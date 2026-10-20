// cloudflare/src/routes/ui.ts
// Server-Rendered UI Routes & Redirects

import type { Env, TaskItem, ChangeRequest, RegisteredAgent, BudgetStatus, BreadcrumbItem, ProjectAnalysis, ProjectRegistryItem } from "../types";
import { renderShell } from "../ui/shell";
import { renderCodePage } from "../ui/pages/code";
import { renderIssuesPage, renderIssueDetailPage } from "../ui/pages/issues";
import { renderPullsPage, renderPullDetailPage, isEligibleForNeedsJudgment } from "../ui/pages/pulls";
import { renderAgentsPage } from "../ui/pages/agents";
import { renderActionsPage, renderActionDetailPage } from "../ui/pages/actions";
import { renderInsightsPage } from "../ui/pages/insights";
import { renderRehearsalsPage, renderRehearsalDetailPage } from "../ui/pages/rehearsals";
import { renderPlatformPage } from "../ui/pages/platform";

const DEFAULT_PROJECT_ID = "brakets";

export async function handleUIRoutes(request: Request, env: Env): Promise<Response | null> {
  const url = new URL(request.url);
  const path = url.pathname;
  const pathParts = path.split("/").filter(Boolean);

  // --------------------------------------------------------------------------
  // 1. Explicit 301 Redirects: /p/s3a-swarm-main/* -> /p/brakets/*
  // --------------------------------------------------------------------------
  if (pathParts[0] === "p" && pathParts[1] === "s3a-swarm-main") {
    const rest = pathParts.slice(2).join("/");
    const dest = rest ? `/p/brakets/${rest}` : `/p/brakets/code`;
    const search = url.search || "";
    return new Response(null, {
      status: 301,
      headers: { Location: `${dest}${search}` }
    });
  }

  // /project/s3a-swarm-main/* (browser requests) -> /p/brakets/*
  if (pathParts[0] === "project" && pathParts[1] === "s3a-swarm-main") {
    const rest = pathParts.slice(2).join("/");
    const dest = rest ? `/p/brakets/${rest}` : `/p/brakets/code`;
    const search = url.search || "";
    return new Response(null, {
      status: 301,
      headers: { Location: `${dest}${search}` }
    });
  }

  // --------------------------------------------------------------------------
  // 2. Swarm Platform Routes (/platform/*)
  // --------------------------------------------------------------------------
  if (pathParts[0] === "platform") {
    const platformSection = (pathParts[1] || "overview") as "overview" | "projects" | "agents" | "ledger" | "traces";
    const doId = env.REPO_COORDINATOR.idFromName("s3a-swarm-main");
    const stub = env.REPO_COORDINATOR.get(doId);

    let budgetData: BudgetStatus | undefined = undefined;
    let agentsList: RegisteredAgent[] = [];
    let projectsList: ProjectRegistryItem[] = [];

    const [resBudget, resAgents, resProjects] = await Promise.allSettled([
      stub.fetch(new Request("https://internal/project/s3a-swarm-main/ledger")),
      stub.fetch(new Request("https://internal/project/s3a-swarm-main/agents")),
      stub.fetch(new Request("https://internal/project/s3a-swarm-main/projects")),
    ]);

    if (resBudget.status === "fulfilled" && resBudget.value.ok) {
      try { budgetData = await resBudget.value.json() as BudgetStatus; } catch (e) {}
    }
    if (resAgents.status === "fulfilled" && resAgents.value.ok) {
      try {
        const aJson: any = await resAgents.value.json();
        agentsList = Object.values(aJson.agents || {});
      } catch (e) {}
    }
    if (resProjects.status === "fulfilled" && resProjects.value.ok) {
      try {
        const pJson: any = await resProjects.value.json();
        projectsList = pJson.projects || [];
      } catch (e) {}
    }
    if (projectsList.length === 0) {
      projectsList = [
        { id: "brakets", name: "Brakets", slug: "plat-gaming", repo_name: "ashleycribb/plat-gaming", description: "High-concurrency esports tournament progression and bracket generation", env: "test", created_at: Date.now(), active_agent: "gate-agent (builder)", active_task: "TASK-283" },
        { id: "scholar", name: "Scholar", slug: "scholar-engine", repo_name: "ashleycribb/scholar-engine", description: "Semantic academic citation graphs, W3C PROV-O provenance, and OWL 2 DL reasoning", env: "demo", created_at: Date.now(), active_agent: "antigravity (reviewer)", active_task: "SCHOLAR-001" },
      ];
    }

    const pageHtml = renderPlatformPage({
      section: platformSection,
      projects: projectsList,
      agents: agentsList,
      budget: budgetData,
    });

    const fullHtml = renderShell(pageHtml, {
      projectId: "platform",
      projectName: "swarm",
      projectSlug: "platform",
      activeTab: platformSection,
      pageTitle: platformSection.charAt(0).toUpperCase() + platformSection.slice(1),
      isPlatform: true,
      projects: projectsList,
      counts: { agents: agentsList.length },
    });

    return new Response(fullHtml, {
      status: 200,
      headers: { "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-cache" },
    });
  }

  // --------------------------------------------------------------------------
  // 3. Retire ?tab= query parameter (301 Redirect) for top-level navigation
  // --------------------------------------------------------------------------
  if (url.searchParams.has("tab") && pathParts.length < 3) {
    const rawTab = url.searchParams.get("tab") || "code";
    let targetTab = rawTab;
    if (["pull-requests", "pr", "prs"].includes(targetTab)) targetTab = "pulls";
    if (!["code", "issues", "pulls", "agents", "actions", "rehearsals", "insights"].includes(targetTab)) {
      targetTab = "code";
    }

    // Determine target project
    let proj = DEFAULT_PROJECT_ID;
    if (pathParts[0] === "p" && pathParts[1]) {
      proj = pathParts[1] === "s3a-swarm-main" ? "brakets" : pathParts[1];
    } else if (pathParts[0] === "project" && pathParts[1]) {
      proj = pathParts[1] === "s3a-swarm-main" ? "brakets" : pathParts[1];
    }

    return new Response(null, {
      status: 301,
      headers: { Location: `/p/${proj}/${targetTab}` }
    });
  }

  // --------------------------------------------------------------------------
  // 4. Retire /preview/cr links (301 Redirect)
  // --------------------------------------------------------------------------
  if (path.includes("/preview/cr/")) {
    const crId = path.split("/preview/cr/")[1]?.split("/")[0] || "";
    let proj = DEFAULT_PROJECT_ID;
    if (pathParts[0] === "p" && pathParts[1]) proj = pathParts[1] === "s3a-swarm-main" ? "brakets" : pathParts[1];
    else if (pathParts[0] === "project" && pathParts[1]) proj = pathParts[1] === "s3a-swarm-main" ? "brakets" : pathParts[1];

    return new Response(null, {
      status: 301,
      headers: { Location: `/p/${proj}/pulls/${crId}` }
    });
  }

  // --------------------------------------------------------------------------
  // 5. Root & Legacy Route Redirects (301)
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
    return new Response(null, { status: 301, headers: { Location: `/platform/agents` } });
  }
  if (pathParts[0] === "actions") {
    const dest = pathParts[1] ? `/p/${DEFAULT_PROJECT_ID}/actions/${pathParts[1]}` : `/p/${DEFAULT_PROJECT_ID}/actions`;
    return new Response(null, { status: 301, headers: { Location: dest } });
  }
  if (pathParts[0] === "rehearsals") {
    const dest = pathParts[1] ? `/p/${DEFAULT_PROJECT_ID}/rehearsals/${pathParts[1]}` : `/p/${DEFAULT_PROJECT_ID}/rehearsals`;
    return new Response(null, { status: 301, headers: { Location: dest } });
  }
  if (pathParts[0] === "insights") {
    return new Response(null, { status: 301, headers: { Location: `/p/${DEFAULT_PROJECT_ID}/insights` } });
  }

  // /project/:id/issues, /project/:id/pulls, etc. (browser requests redirect to /p/:id/...)
  if (pathParts[0] === "project" && pathParts.length >= 2) {
    const proj = pathParts[1] === "s3a-swarm-main" ? "brakets" : pathParts[1];
    const sub = pathParts[2];
    if (sub && ["code", "issues", "pulls", "agents", "actions", "rehearsals", "insights"].includes(sub)) {
      const rest = pathParts.slice(3).join("/");
      const dest = rest ? `/p/${proj}/${sub}/${rest}` : `/p/${proj}/${sub}`;
      return new Response(null, { status: 301, headers: { Location: dest } });
    }
    if (pathParts.length === 2) {
      return new Response(null, { status: 301, headers: { Location: `/p/${proj}/code` } });
    }
  }

  // --------------------------------------------------------------------------
  // 6. Canonical /p/:projectId/... Server-Rendered UI Routes
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
  if (!["code", "issues", "pulls", "agents", "actions", "rehearsals", "insights"].includes(section)) {
    return null;
  }

  // Fetch project data from Durable Object
  const coordinatorId = (projectId === "brakets" || projectId === "s3a-swarm-main") ? "s3a-swarm-main" : projectId;
  const doId = env.REPO_COORDINATOR.idFromName(coordinatorId);
  const stub = env.REPO_COORDINATOR.get(doId);

  let statusData: any = {};
  let controlData: any = { paused: false };
  let tasks: TaskItem[] = [];
  let crs: ChangeRequest[] = [];
  let registeredAgentsList: RegisteredAgent[] = [];
  let budgetData: BudgetStatus | undefined = undefined;
  let analysisData: ProjectAnalysis | undefined = undefined;
  let projectsList: ProjectRegistryItem[] = [];

  const [resStatus, resControl, resBudget, resTasks, resCRs, resAgents, resAnalysis, resProjects] = await Promise.allSettled([
    stub.fetch(new Request(`https://internal/project/${coordinatorId}/status`)),
    stub.fetch(new Request(`https://internal/project/${coordinatorId}/control`)),
    stub.fetch(new Request(`https://internal/project/${coordinatorId}/ledger`)),
    stub.fetch(new Request(`https://internal/project/${coordinatorId}/tasks`)),
    stub.fetch(new Request(`https://internal/project/${coordinatorId}/change-requests`)),
    stub.fetch(new Request(`https://internal/project/${coordinatorId}/agents`)),
    stub.fetch(new Request(`https://internal/project/${coordinatorId}/analysis`)),
    stub.fetch(new Request(`https://internal/project/${coordinatorId}/projects`)),
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
  if (resAnalysis.status === "fulfilled" && resAnalysis.value.ok) {
    try {
      analysisData = await resAnalysis.value.json() as ProjectAnalysis;
    } catch (e) {}
  }
  if (resProjects.status === "fulfilled" && resProjects.value.ok) {
    try {
      const pJson: any = await resProjects.value.json();
      projectsList = pJson.projects || [];
    } catch (e) {}
  }
  if (projectsList.length === 0) {
    projectsList = [
      { id: "brakets", name: "Brakets", slug: "plat-gaming", repo_name: "ashleycribb/plat-gaming", description: "High-concurrency esports tournament progression and bracket generation", env: (statusData.brakets_env as any) || "demo", created_at: Date.now(), active_agent: "gate-agent (builder)", active_task: "TASK-283" },
      { id: "scholar", name: "Scholar", slug: "scholar-engine", repo_name: "ashleycribb/scholar-engine", description: "Semantic academic citation graphs, W3C PROV-O provenance, and OWL 2 DL reasoning", env: "demo", created_at: Date.now(), active_agent: "antigravity (reviewer)", active_task: "SCHOLAR-001" },
    ];
  }

  const counts = {
    issues: tasks.filter(t => t.status !== "merged").length,
    pulls: crs.filter(c => isEligibleForNeedsJudgment(c, crs)).length,
    agents: registeredAgentsList.length,
  };
  const isPaused = controlData.paused ?? false;
  const pausedBy = controlData.paused_by || statusData.paused_by;
  const pausedReason = controlData.paused_reason || statusData.paused_reason;
  const pausedAt = controlData.paused_at || statusData.paused_at;

  let pageHtml = "";
  let pageTitle = "";
  let breadcrumbs: BreadcrumbItem[] = [];

  switch (section) {
    case "code": {
      pageTitle = "Code Analysis";
      breadcrumbs = [{ label: "Code Analysis" }];
      pageHtml = renderCodePage({ projectId, statusData, analysisData });
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
        const activeFilter = url.searchParams.get("tab") || url.searchParams.get("filter") || "needs-judgment";
        pageHtml = renderPullsPage({ projectId, changeRequests: crs, filter: activeFilter });
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

    case "rehearsals": {
      if (detailId) {
        let selectedRun = null;
        try {
          const runRes = await stub.fetch(new Request(`https://internal/project/${coordinatorId}/rehearsals/${detailId}`));
          if (runRes.ok) selectedRun = await runRes.json();
        } catch (e) {}
        pageTitle = selectedRun ? `Rehearsal Run · ${(selectedRun as any).run_id}` : `Rehearsal Not Found · ${detailId}`;
        breadcrumbs = [
          { label: "Rehearsals", url: `/p/${projectId}/rehearsals` },
          { label: detailId }
        ];
        pageHtml = selectedRun ? renderRehearsalDetailPage(projectId, selectedRun as any) : `
          <div style="text-align: center; padding: 3rem 1rem;">
            <h2 style="color: var(--gh-text-heading); font-size: 1.2rem; margin-bottom: 0.5rem;">Rehearsal Run Not Found</h2>
            <p style="color: var(--gh-text-muted); font-size: 13px;">The requested rehearsal run <code>${detailId}</code> could not be located in storage.</p>
            <a href="/p/${projectId}/rehearsals" class="gh-btn" style="margin-top: 1rem;">Back to Rehearsals List</a>
          </div>
        `;
      } else {
        let runsList: any[] = [];
        try {
          const runsRes = await stub.fetch(new Request(`https://internal/project/${coordinatorId}/rehearsals`));
          if (runsRes.ok) {
            const data: any = await runsRes.json();
            runsList = data.runs || [];
          }
        } catch (e) {}
        pageTitle = "Rehearsals";
        breadcrumbs = [{ label: "Rehearsals" }];
        pageHtml = renderRehearsalsPage(projectId, runsList, { isPaused, tasks });
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

  const braketsEnv = (statusData.brakets_env as any) || "demo";
  const projMeta = projectsList.find(p => p.id === projectId);
  const projectName = projMeta?.name || (projectId === "brakets" || projectId === "s3a-swarm-main" ? "Brakets" : (projectId === "scholar" ? "Scholar" : projectId));
  const projectSlug = projMeta?.slug || (projectId === "brakets" || projectId === "s3a-swarm-main" ? "plat-gaming" : (projectId === "scholar" ? "scholar-engine" : projectId));
  const activeAgentInfo = projMeta?.active_agent 
    ? `${projMeta.active_agent} working on ${projMeta.active_task || 'Backlog'}`
    : "gate-agent (builder) working on TASK-283";

  const fullHtml = renderShell(pageHtml, {
    projectId,
    projectName,
    projectSlug,
    activeTab: section as any,
    pageTitle,
    breadcrumbs,
    counts,
    isPaused,
    pausedBy,
    pausedReason,
    pausedAt,
    braketsEnv,
    projects: projectsList,
    activeAgentInfo,
  });

  return new Response(fullHtml, {
    status: 200,
    headers: {
      "Content-Type": "text/html; charset=utf-8",
      "Cache-Control": "no-cache",
    },
  });
}
