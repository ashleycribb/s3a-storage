// cloudflare/src/ui/shell.ts
// Shared Application Shell: Common Layout, Navigation, Breadcrumbs, Modals, and Client Scripts

import type { BreadcrumbItem, BraketsEnv, ProjectRegistryItem } from "../types";

export interface ShellOptions {
  projectId: string;
  projectName?: string;
  projectSlug?: string;
  activeTab: string;
  pageTitle?: string;
  breadcrumbs?: BreadcrumbItem[];
  counts?: {
    issues?: number;
    pulls?: number;
    agents?: number;
  };
  isPaused?: boolean;
  pausedBy?: string;
  pausedReason?: string;
  pausedAt?: number;
  braketsEnv?: BraketsEnv;
  projects?: ProjectRegistryItem[];
  activeAgentInfo?: string;
  isPlatform?: boolean;
}

export function renderShell(contentHtml: string, options: ShellOptions): string {
  const {
    projectId,
    activeTab,
    breadcrumbs = [],
    counts = {},
    isPaused = false,
    pausedBy,
    pausedReason,
    pausedAt,
    braketsEnv = "demo",
    isPlatform = false,
    projects = [],
  } = options;

  const projectName = options.projectName || (projectId === "scholar" ? "Scholar" : (projectId === "brakets" || projectId === "s3a-swarm-main" ? "Brakets" : projectId));
  const projectSlug = options.projectSlug || (projectId === "scholar" ? "scholar-engine" : (projectId === "brakets" || projectId === "s3a-swarm-main" ? "plat-gaming" : projectId));
  const activeAgentInfo = options.activeAgentInfo || (projectId === "scholar" ? "scholar-agent (librarian) working on SCHOLAR-001" : "gate-agent (builder) working on TASK-283");

  const titlePrefix = options.pageTitle ? `${options.pageTitle} · ` : "";
  const fullTitle = isPlatform
    ? `${titlePrefix}swarm · AI Agentic GitHub Platform`
    : `${titlePrefix}swarm / ${projectName} (${projectSlug})`;

  // Render breadcrumbs
  const breadcrumbItems = isPlatform
    ? [{ label: "swarm", url: "/platform" }, ...breadcrumbs]
    : [
        { label: projectName, url: `/p/${projectId}/code` },
        ...breadcrumbs
      ];

  const breadcrumbsHtml = breadcrumbItems.map((item, idx) => {
    const isLast = idx === breadcrumbItems.length - 1;
    if (isLast || !item.url) {
      return `<span style="color: var(--gh-text-heading); font-weight: 600;">${item.label}</span>`;
    }
    return `<a href="${item.url}" style="color: var(--gh-link); text-decoration: none;">${item.label}</a>`;
  }).join('<span style="color: var(--gh-text-muted); margin: 0 0.35rem;">/</span>');

  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>${fullTitle.replace(/"/g, '&quot;')}</title>
  <style>
    :root {
      --gh-bg: #0d1117;
      --gh-card-bg: #161b22;
      --gh-border: #30363d;
      --gh-border-subtle: #21262d;
      --gh-text: #c9d1d9;
      --gh-text-heading: #f0f6fc;
      --gh-text-muted: #8b949e;
      --gh-link: #58a6ff;
      --gh-green: #238636;
      --gh-green-hover: #2ea043;
      --gh-green-text: #3fb950;
      --gh-green-bg: rgba(35, 134, 54, 0.15);
      --gh-red: #da3633;
      --gh-red-text: #f85149;
      --gh-red-bg: rgba(218, 54, 51, 0.15);
      --gh-purple: #8957e5;
      --gh-purple-text: #a371f7;
      --gh-purple-bg: rgba(137, 87, 229, 0.15);
      --gh-yellow: #d29922;
      --gh-yellow-text: #e3b341;
      --gh-yellow-bg: rgba(210, 153, 34, 0.15);
      --gh-blue: #1f6feb;
      --gh-blue-text: #58a6ff;
      --gh-blue-bg: rgba(56, 139, 253, 0.15);
      --gh-orange: #f38020;
    }
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "Noto Sans", Helvetica, Arial, sans-serif;
      background: var(--gh-bg);
      color: var(--gh-text);
      line-height: 1.5;
      font-size: 14px;
      min-height: 100vh;
      display: flex;
      flex-direction: column;
    }
    a { color: var(--gh-link); text-decoration: none; }
    a:hover { text-decoration: underline; }

    /* Top Navigation Header */
    .gh-header {
      background: var(--gh-card-bg);
      border-bottom: 1px solid var(--gh-border);
      padding: 0.85rem 1.5rem 0 1.5rem;
    }
    .gh-header-top {
      display: flex;
      justify-content: space-between;
      align-items: center;
      flex-wrap: wrap;
      gap: 1rem;
      margin-bottom: 0.85rem;
    }
    .repo-title-row {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      font-size: 1.15rem;
      color: var(--gh-text-heading);
      flex-wrap: wrap;
    }
    .repo-pill {
      font-size: 0.72rem;
      font-weight: 600;
      padding: 0.12rem 0.5rem;
      border-radius: 2em;
      border: 1px solid var(--gh-border);
      color: var(--gh-text-muted);
    }

    .env-badge-demo {
      background: rgba(210, 153, 34, 0.18) !important;
      color: #e3b341 !important;
      border: 1px solid rgba(210, 153, 34, 0.5) !important;
    }
    .env-badge-test {
      background: rgba(56, 139, 253, 0.18) !important;
      color: #58a6ff !important;
      border: 1px solid rgba(56, 139, 253, 0.5) !important;
    }
    .env-badge-production {
      background: rgba(46, 160, 67, 0.2) !important;
      color: #3fb950 !important;
      border: 1px solid rgba(46, 160, 67, 0.6) !important;
    }

    /* Buttons */
    .gh-btn {
      display: inline-flex;
      align-items: center;
      gap: 0.35rem;
      padding: 5px 12px;
      font-size: 13px;
      font-weight: 500;
      line-height: 20px;
      border-radius: 6px;
      border: 1px solid var(--gh-border);
      background: #21262d;
      color: var(--gh-text);
      cursor: pointer;
      text-decoration: none;
      transition: background 0.15s, opacity 0.15s;
    }
    .gh-btn:hover:not(:disabled) { background: #30363d; text-decoration: none; }
    .gh-btn:disabled { opacity: 0.6; cursor: not-allowed; }
    .gh-btn-primary {
      background: var(--gh-green);
      color: #fff;
      border-color: rgba(240, 246, 252, 0.1);
    }
    .gh-btn-primary:hover:not(:disabled) { background: var(--gh-green-hover); }
    .gh-btn-danger {
      background: #21262d;
      color: var(--gh-red-text);
      border-color: rgba(248, 81, 73, 0.4);
    }
    .gh-btn-danger:hover:not(:disabled) { background: var(--gh-red); color: #fff; }
    .gh-btn-warning {
      background: #21262d;
      color: var(--gh-yellow-text);
      border-color: rgba(210, 153, 34, 0.4);
    }
    .gh-btn-warning:hover:not(:disabled) { background: var(--gh-yellow); color: #0d1117; }
    .gh-btn-sm { padding: 3px 8px; font-size: 12px; line-height: 18px; }

    /* Inputs */
    .gh-input, .gh-textarea, .gh-select {
      background: #0d1117;
      border: 1px solid var(--gh-border);
      border-radius: 6px;
      color: var(--gh-text-heading);
      padding: 6px 10px;
      font-size: 13px;
      font-family: inherit;
      width: 100%;
    }
    .gh-input:focus, .gh-textarea:focus, .gh-select:focus {
      outline: none;
      border-color: var(--gh-link);
      box-shadow: 0 0 0 3px rgba(88, 166, 255, 0.2);
    }

    /* Nav Tabs */
    .gh-tabs {
      display: flex;
      gap: 0.35rem;
      overflow-x: auto;
      scrollbar-width: none;
    }
    .gh-tabs::-webkit-scrollbar { display: none; }
    .tab-item {
      display: inline-flex;
      align-items: center;
      gap: 0.4rem;
      padding: 0.5rem 0.85rem;
      color: var(--gh-text);
      font-size: 14px;
      font-weight: 500;
      border-bottom: 2px solid transparent;
      white-space: nowrap;
      text-decoration: none;
    }
    .tab-item:hover { color: var(--gh-text-heading); text-decoration: none; }
    .tab-item.active {
      color: var(--gh-text-heading);
      border-bottom: 2px solid #f78166;
      font-weight: 600;
    }
    .tab-item svg { fill: currentColor; }
    .tab-counter {
      background: rgba(110, 118, 129, 0.4);
      color: var(--gh-text);
      font-size: 11px;
      font-weight: 600;
      padding: 0.1rem 0.45rem;
      border-radius: 2em;
    }

    /* Page Container */
    .gh-container {
      max-width: 1280px;
      margin: 1.5rem auto;
      padding: 0 1.5rem;
      width: 100%;
      flex: 1;
    }

    /* Card Panels & Tables */
    .gh-card {
      background: var(--gh-card-bg);
      border: 1px solid var(--gh-border);
      border-radius: 6px;
      overflow: hidden;
      margin-bottom: 1.5rem;
    }
    .gh-card-header {
      background: #161b22;
      padding: 0.75rem 1rem;
      border-bottom: 1px solid var(--gh-border);
      display: flex;
      justify-content: space-between;
      align-items: center;
      flex-wrap: wrap;
      gap: 0.5rem;
    }
    .gh-card-body { padding: 1rem; }
    .gh-row {
      display: flex;
      justify-content: space-between;
      align-items: center;
      padding: 0.75rem 1rem;
      border-bottom: 1px solid var(--gh-border-subtle);
      transition: background 0.1s;
    }
    .gh-row:last-child { border-bottom: none; }
    .gh-row:hover { background: rgba(177, 186, 196, 0.04); }

    /* Badges */
    .gh-badge {
      display: inline-flex;
      align-items: center;
      gap: 0.3rem;
      padding: 0.15rem 0.55rem;
      border-radius: 2em;
      font-size: 11px;
      font-weight: 600;
      line-height: 16px;
      border: 1px solid transparent;
      white-space: nowrap;
    }
    .gh-badge-success { background: var(--gh-green-bg); color: var(--gh-green-text); border-color: rgba(63, 185, 80, 0.4); }
    .gh-badge-danger { background: var(--gh-red-bg); color: var(--gh-red-text); border-color: rgba(248, 81, 73, 0.4); }
    .gh-badge-purple { background: var(--gh-purple-bg); color: var(--gh-purple-text); border-color: rgba(163, 113, 247, 0.4); }
    .gh-badge-yellow { background: var(--gh-yellow-bg); color: var(--gh-yellow-text); border-color: rgba(227, 179, 65, 0.4); }
    .gh-badge-blue { background: var(--gh-blue-bg); color: var(--gh-blue-text); border-color: rgba(88, 166, 255, 0.4); }
    .gh-badge-muted { background: rgba(110, 118, 129, 0.15); color: var(--gh-text-muted); border-color: var(--gh-border); }

    /* Empty and Error States */
    .gh-blankslate {
      text-align: center;
      padding: 3.5rem 1.5rem;
      background: var(--gh-card-bg);
      border: 1px dashed var(--gh-border);
      border-radius: 6px;
    }
    .gh-blankslate-icon {
      font-size: 2.2rem;
      margin-bottom: 0.75rem;
      opacity: 0.8;
    }
    .gh-blankslate h3 {
      font-size: 1.15rem;
      font-weight: 600;
      color: var(--gh-text-heading);
      margin-bottom: 0.5rem;
    }
    .gh-blankslate p {
      color: var(--gh-text-muted);
      font-size: 13px;
      max-width: 480px;
      margin: 0 auto 1.25rem auto;
    }

    /* Modal dialogs */
    .modal-overlay {
      position: fixed;
      top: 0; left: 0; right: 0; bottom: 0;
      background: rgba(1, 4, 9, 0.85);
      backdrop-filter: blur(4px);
      display: none;
      align-items: center;
      justify-content: center;
      z-index: 9999;
      padding: 1.5rem;
    }
  </style>
</head>
<body>
  <!-- Header with Project Switcher, Breadcrumbs & Global Action Buttons -->
  <header class="gh-header">
    <div class="gh-header-top">
      <div class="repo-title-row">
        <!-- Swarm Platform Logo & Link -->
        <a href="/platform" style="text-decoration: none; display: inline-flex; align-items: center; gap: 0.35rem; color: #f0f6fc; font-weight: 700;">
          <span style="color: var(--gh-orange); font-size: 1.25rem; font-weight: 800;">swarm</span>
        </a>

        ${isPlatform ? `
          <span style="color: var(--gh-text-muted); margin: 0 0.15rem;">/</span>
          <span style="color: var(--gh-text-heading); font-weight: 600;">${options.pageTitle || 'Platform'}</span>
          <span class="repo-pill" style="border-color: rgba(243,128,32,0.4); color: var(--gh-orange);">Autonomous Platform</span>
        ` : `
          <span style="color: var(--gh-text-muted); font-size: 12px; margin: 0 0.15rem;">▶</span>

          <!-- Project Switcher Dropdown Trigger -->
          <div style="position: relative; display: inline-block;">
            <button onclick="toggleProjectDropdown()" id="project-switcher-btn" class="gh-btn gh-btn-sm" style="background: #21262d; color: var(--gh-text-heading); font-weight: 600; font-size: 13px; border: 1px solid var(--gh-border); display: inline-flex; align-items: center; gap: 0.4rem; padding: 3px 8px; cursor: pointer;">
              <span style="color: var(--gh-link);">${projectName} (${projectSlug})</span>
              <span style="font-size: 9px; color: var(--gh-text-muted);">▼</span>
            </button>

            <!-- Project Switcher Dropdown Menu -->
            <div id="project-dropdown-menu" style="display: none; position: absolute; top: 100%; left: 0; margin-top: 5px; background: #161b22; border: 1px solid var(--gh-border); border-radius: 6px; box-shadow: 0 10px 30px rgba(0,0,0,0.8); min-width: 290px; z-index: 9999; overflow: hidden;">
              <div style="padding: 7px 12px; font-size: 11px; font-weight: 700; color: var(--gh-text-muted); background: #0d1117; border-bottom: 1px solid var(--gh-border); text-transform: uppercase;">
                Switch Swarm Project
              </div>
              <div style="max-height: 280px; overflow-y: auto;">
                ${(projects.length > 0 ? projects : [
                  { id: "brakets", name: "Brakets", slug: "plat-gaming", description: "Esports tournament progression engine", env: braketsEnv },
                  { id: "scholar", name: "Scholar", slug: "scholar-engine", description: "Academic citation graphs and OWL reasoning", env: "demo" }
                ]).map(p => `
                  <a href="/p/${p.id}/code" style="display: flex; justify-content: space-between; align-items: center; padding: 8px 12px; text-decoration: none; color: var(--gh-text); border-bottom: 1px solid var(--gh-border-subtle); ${p.id === projectId ? 'background: rgba(56, 139, 253, 0.15); color: #58a6ff;' : ''}">
                    <div>
                      <div style="font-weight: 600; font-size: 13px;">${p.name} <span style="font-size: 11px; color: var(--gh-text-muted);">(${p.slug})</span></div>
                      <div style="font-size: 11px; color: var(--gh-text-muted);">${(p.description || '').slice(0, 34)}...</div>
                    </div>
                    ${p.id === projectId ? '<span style="color: #3fb950; font-weight: 700; font-size: 14px;">✓</span>' : ''}
                  </a>
                `).join('')}
              </div>
              <div style="padding: 6px 8px; background: #0d1117; display: flex; flex-direction: column; gap: 4px; border-top: 1px solid var(--gh-border);">
                <button type="button" onclick="openCreateProjectModal()" class="gh-btn gh-btn-sm gh-btn-primary" style="width: 100%; justify-content: center; font-size: 12px;">
                  + Create New Project
                </button>
                <a href="/platform" class="gh-btn gh-btn-sm" style="width: 100%; justify-content: center; font-size: 12px;">
                  ⚙️ Swarm Platform Overview
                </a>
              </div>
            </div>
          </div>

          <span style="color: var(--gh-text-muted); margin: 0 0.15rem;">•</span>
          <span id="env-badge" class="repo-pill env-badge env-badge-${braketsEnv}" onclick="openPromotionModal()" style="cursor: pointer; font-weight: 700; text-transform: uppercase;" title="Project Environment: ${braketsEnv.toUpperCase()}. Click to view promotion status & checklist.">
            ${braketsEnv === 'production' ? '🛡️' : (braketsEnv === 'test' ? '🧪' : '🎮')} ENV: ${braketsEnv}
          </span>

          <!-- Active Agent & Task Indicator ("who is working on what") -->
          <span class="repo-pill" style="border-color: rgba(56, 139, 253, 0.4); color: #58a6ff; font-weight: 500;" title="Active Agent activity on ${projectName}">
            🤖 ${activeAgentInfo}
          </span>
        `}
      </div>

      <!-- Action buttons -->
      <div style="display: flex; gap: 0.5rem; align-items: center; flex-wrap: wrap;">
        <button id="btn-create-project-top" class="gh-btn gh-btn-primary" onclick="openCreateProjectModal()" title="Create a new isolated project in Swarm">
          + New Project
        </button>
        ${!isPlatform ? `
          <button id="btn-promote-env" class="gh-btn" onclick="openPromotionModal()" title="View environment lifecycle & promotion checklist">
            🚀 Promote (${braketsEnv})
          </button>
          <button id="engine-toggle-btn" class="gh-btn ${isPaused ? 'gh-btn-primary' : 'gh-btn-danger'}" onclick="toggleAutonomousEngine('${isPaused ? 'resume' : 'pause'}')">
            ${isPaused ? '▶ Resume Swarm' : '⏸ Pause Swarm'}
          </button>
          ${isPaused && pausedReason ? `
            <span style="font-size: 11px; color: var(--gh-yellow-text); background: rgba(210,153,34,0.15); border: 1px solid rgba(210,153,34,0.3); border-radius: 4px; padding: 3px 8px; font-weight: 500;" title="Paused by ${pausedBy || 'Supervisor'}">
              ⏸ Paused by ${pausedBy || 'Supervisor'}: ${pausedReason}
            </span>
          ` : ''}
          <button id="btn-manual-step" class="gh-btn" onclick="triggerManualTick()">
            ⚡ Run 1 Step
          </button>
          <button class="gh-btn" onclick="openEnrollmentModal()">
            🔑 Enroll Key
          </button>
        ` : `
          <a href="/p/brakets/code" class="gh-btn">
            Open Brakets ↗
          </a>
        `}
      </div>
    </div>

    <!-- Navigation Tabs -->
    <nav class="gh-tabs" aria-label="${isPlatform ? 'Platform' : 'Repository'}">
      ${isPlatform ? `
        <a href="/platform" class="tab-item ${activeTab === 'overview' ? 'active' : ''}">
          <span>Overview</span>
        </a>
        <a href="/platform/projects" class="tab-item ${activeTab === 'projects' ? 'active' : ''}">
          <span>Projects (${projects.length})</span>
        </a>
        <a href="/platform/agents" class="tab-item ${activeTab === 'agents' ? 'active' : ''}">
          <span>Agents (${counts.agents || 2})</span>
        </a>
        <a href="/platform/ledger" class="tab-item ${activeTab === 'ledger' ? 'active' : ''}">
          <span>Ledger</span>
        </a>
        <a href="/platform/traces" class="tab-item ${activeTab === 'traces' ? 'active' : ''}">
          <span>Traces</span>
        </a>
      ` : `
        <a href="/p/${projectId}/code" id="nav-tab-code" class="tab-item ${activeTab === 'code' ? 'active' : ''}">
          <svg height="16" viewBox="0 0 16 16" width="16"><path fill-rule="evenodd" d="M4.72 3.22a.75.75 0 011.06 1.06L2.06 8l3.72 3.72a.75.75 0 11-1.06 1.06L.47 8.53a.75.75 0 010-1.06l4.25-4.25zm6.56 0a.75.75 0 10-1.06 1.06L13.94 8l-3.72 3.72a.75.75 0 101.06 1.06l4.25-4.25a.75.75 0 000-1.06l-4.25-4.25z"/></svg>
          <span>Code</span>
        </a>
        <a href="/p/${projectId}/issues" id="nav-tab-issues" class="tab-item ${activeTab === 'issues' ? 'active' : ''}">
          <svg height="16" viewBox="0 0 16 16" width="16"><path d="M8 9.5a1.5 1.5 0 100-3 1.5 1.5 0 000 3z"/><path fill-rule="evenodd" d="M8 0a8 8 0 100 16A8 8 0 008 0zM1.5 8a6.5 6.5 0 1113 0 6.5 6.5 0 01-13 0z"/></svg>
          <span>Issues</span>
          ${counts.issues !== undefined ? `<span class="tab-counter">${counts.issues}</span>` : ''}
        </a>
        <a href="/p/${projectId}/pulls" id="nav-tab-pulls" class="tab-item ${activeTab === 'pulls' ? 'active' : ''}">
          <svg height="16" viewBox="0 0 16 16" width="16"><path fill-rule="evenodd" d="M7.177 3.073L9.573.677A.25.25 0 0110 .854v4.792a.25.25 0 01-.427.177L7.177 3.427a.25.25 0 010-.354zM3.75 2.5a.75.75 0 100 1.5.75.75 0 000-1.5zm-2.25.75a2.25 2.25 0 113 2.122v5.256a2.251 2.251 0 11-1.5 0V5.372A2.25 2.25 0 011.5 3.25zM11 2.5h-1V4h1a1 1 0 011 1v5.628a2.251 2.251 0 101.5 0V5A2.5 2.5 0 0011 2.5zm1 10.25a.75.75 0 111.5 0 .75.75 0 01-1.5 0zM3.75 12a.75.75 0 100 1.5.75.75 0 000-1.5z"/></svg>
          <span>Pull requests</span>
          ${counts.pulls !== undefined ? `<span class="tab-counter">${counts.pulls}</span>` : ''}
        </a>
        <a href="/p/${projectId}/agents" id="nav-tab-agents" class="tab-item ${activeTab === 'agents' ? 'active' : ''}">
          <svg height="16" viewBox="0 0 16 16" width="16"><path d="M10.561 8.073a6.005 6.005 0 013.432 5.142.75.75 0 11-1.498.07 4.5 4.5 0 00-8.99 0 .75.75 0 01-1.498-.07 6.004 6.004 0 013.431-5.142 3.999 3.999 0 115.123 0zM10.5 5a2.5 2.5 0 10-5 0 2.5 2.5 0 005 0z"/></svg>
          <span>Agents</span>
          ${counts.agents !== undefined ? `<span class="tab-counter">${counts.agents}</span>` : ''}
        </a>
        <a href="/p/${projectId}/actions" id="nav-tab-actions" class="tab-item ${activeTab === 'actions' ? 'active' : ''}">
          <svg height="16" viewBox="0 0 16 16" width="16"><path fill-rule="evenodd" d="M1.5 8a6.5 6.5 0 1113 0 6.5 6.5 0 01-13 0zM0 8a8 8 0 1116 0A8 8 0 010 8zm6.5-2.75A.75.75 0 017.25 4.5h1.5a.75.75 0 01.75.75v5.5a.75.75 0 01-.75.75h-1.5a.75.75 0 01-.75-.75v-5.5z"/></svg>
          <span>Actions</span>
        </a>
        <a href="/p/${projectId}/rehearsals" id="nav-tab-rehearsals" class="tab-item ${activeTab === 'rehearsals' ? 'active' : ''}">
          <svg height="16" viewBox="0 0 16 16" width="16"><path d="M9.504.43a1.5 1.5 0 011.264.508l4.296 4.697c.3.327.436.772.373 1.214l-1.393 9.754A1.5 1.5 0 0112.558 18H3.442a1.5 1.5 0 01-1.486-1.397L.563 6.85a1.5 1.5 0 01.373-1.215L5.232.938A1.5 1.5 0 016.496.43h3.008zM6.5 2.5a.5.5 0 00-.5.5v2a.5.5 0 001 0V3h2v2a.5.5 0 001 0V3a.5.5 0 00-.5-.5h-3zM3 8a1 1 0 100 2 1 1 0 000-2zm10 0a1 1 0 100 2 1 1 0 000-2zm-5 4a1.5 1.5 0 100 3 1.5 1.5 0 000-3z"/></svg>
          <span>Rehearsals</span>
        </a>
        <a href="/p/${projectId}/insights" id="nav-tab-insights" class="tab-item ${activeTab === 'insights' ? 'active' : ''}">
          <svg height="16" viewBox="0 0 16 16" width="16"><path d="M1.5 1.75a.75.75 0 00-1.5 0v12.5c0 .414.336.75.75.75h14.5a.75.75 0 000-1.5H1.5V1.75z"/><path d="M3.75 6.5a.75.75 0 01.75.75v4.5a.75.75 0 01-1.5 0v-4.5a.75.75 0 01.75-.75zm4.5-2a.75.75 0 01.75.75v6.5a.75.75 0 01-1.5 0v-6.5a.75.75 0 01.75-.75zm4.5 4a.75.75 0 01.75.75v2.5a.75.75 0 01-1.5 0v-2.5a.75.75 0 01.75-.75z"/></svg>
          <span>Insights</span>
        </a>
      `}
    </nav>
  </header>

  <!-- Page Content Body -->
  <main class="gh-container">
    ${contentHtml}
  </main>

  <!-- Modal: Supervisor Key Enrollment -->
  <div id="enrollment-modal" class="modal-overlay">
    <div style="background: #161b22; border: 1px solid var(--gh-border); border-radius: 8px; width: 100%; max-width: 650px; overflow: hidden; box-shadow: 0 20px 40px rgba(0,0,0,0.85);">
      <div style="padding: 1rem 1.25rem; border-bottom: 1px solid var(--gh-border); display: flex; justify-content: space-between; align-items: center; background: #0d1117;">
        <div style="display: flex; align-items: center; gap: 0.6rem;">
          <strong style="color: #f0f6fc; font-size: 1.05rem;">🔑 Supervisor Key Enrollment</strong>
          <span class="gh-badge gh-badge-blue">Ed25519 WebCrypto</span>
        </div>
        <button onclick="closeEnrollmentModal()" style="background: transparent; border: none; color: var(--gh-text-muted); font-size: 1.2rem; cursor: pointer;">✕</button>
      </div>
      <div style="padding: 1.25rem; font-size: 13px;">
        <p style="color: var(--gh-text-muted); margin-bottom: 1rem; line-height: 1.5;">
          Human approvals require a valid Ed25519 signature from a public key authorized in the Worker secret <code>SUPERVISOR_PUBKEYS</code>. Auto-registration is disabled.
        </p>

        <div style="margin-bottom: 1rem;">
          <label style="display: block; font-weight: 600; margin-bottom: 0.35rem; color: var(--gh-text-heading);">Your Browser's Ed25519 Public Key (Hex):</label>
          <div style="display: flex; gap: 0.5rem;">
            <input type="text" id="enrollment-pubkey-input" readonly style="flex: 1; background: #0d1117; border: 1px solid var(--gh-border); color: #7ee787; font-family: monospace; font-size: 12px; padding: 0.5rem 0.75rem; border-radius: 6px;">
            <button class="gh-btn gh-btn-sm" onclick="copyEnrollmentPubkey()">📋 Copy</button>
          </div>
        </div>

        <div style="margin-bottom: 1.25rem;">
          <label style="display: block; font-weight: 600; margin-bottom: 0.35rem; color: var(--gh-text-heading);">Worker Authorization Status:</label>
          <div id="enrollment-status-badge">
            <span class="gh-badge gh-badge-muted">Checking authorization...</span>
          </div>
        </div>

        <div style="background: #0d1117; border: 1px solid var(--gh-border); border-radius: 6px; padding: 1rem; margin-bottom: 1.25rem;">
          <div style="font-weight: 600; margin-bottom: 0.35rem; color: var(--gh-orange);">How to Register in Worker Secret:</div>
          <p style="color: var(--gh-text-muted); font-size: 12px; margin-bottom: 0.5rem;">Run the following command in terminal to enroll this public key into the Worker secret:</p>
          <pre style="background: #030712; padding: 0.6rem 0.8rem; border-radius: 4px; font-family: monospace; font-size: 12px; color: #38bdf8; overflow-x: auto; margin: 0;">npx wrangler secret put SUPERVISOR_PUBKEYS</pre>
          <p style="color: var(--gh-text-muted); font-size: 11px; margin-top: 0.5rem;">Paste the public key above. If multiple human supervisors exist, separate keys with commas.</p>
        </div>

        <div style="display: flex; justify-content: flex-end; gap: 0.5rem;">
          <button class="gh-btn" onclick="closeEnrollmentModal()">Close</button>
        </div>
      </div>
    </div>
  </div>

  <!-- Promotion & Environment Lifecycle Modal -->
  <div id="promotion-modal" class="modal-overlay" onclick="if(event.target === this) closePromotionModal()">
    <div style="background: var(--gh-card-bg); border: 1px solid var(--gh-border); border-radius: 8px; width: 100%; max-width: 680px; box-shadow: 0 12px 28px rgba(0,0,0,0.5); overflow: hidden; max-height: 90vh; display: flex; flex-direction: column;">
      <div style="padding: 1rem 1.25rem; border-bottom: 1px solid var(--gh-border); display: flex; justify-content: space-between; align-items: center; background: #0d1117;">
        <h3 style="font-size: 15px; font-weight: 600; color: var(--gh-text-heading); display: flex; align-items: center; gap: 0.5rem;">
          <span>🚀 Brakets Environment Promotion</span>
        </h3>
        <button onclick="closePromotionModal()" style="background: none; border: none; color: var(--gh-text-muted); font-size: 18px; cursor: pointer;">✕</button>
      </div>

      <div style="padding: 1.25rem; overflow-y: auto; flex: 1;">
        <!-- Lifecycle Pipeline -->
        <div style="display: flex; align-items: center; justify-content: space-around; background: #0d1117; border: 1px solid var(--gh-border); border-radius: 6px; padding: 0.85rem 1rem; margin-bottom: 1.25rem;">
          <div style="text-align: center; opacity: ${braketsEnv === 'demo' ? '1' : '0.55'};">
            <span class="gh-badge env-badge-demo" style="font-size: 12px; padding: 0.25rem 0.65rem;">🎮 DEMO</span>
            <div style="font-size: 11px; color: var(--gh-text-muted); margin-top: 0.25rem;">Synthetic Rehearsal</div>
          </div>
          <span style="color: var(--gh-border); font-size: 18px; font-weight: bold;">──▶</span>
          <div style="text-align: center; opacity: ${braketsEnv === 'test' ? '1' : '0.55'};">
            <span class="gh-badge env-badge-test" style="font-size: 12px; padding: 0.25rem 0.65rem;">🧪 TEST</span>
            <div style="font-size: 11px; color: var(--gh-text-muted); margin-top: 0.25rem;">Staging & Verified QA</div>
          </div>
          <span style="color: var(--gh-border); font-size: 18px; font-weight: bold;">──▶</span>
          <div style="text-align: center; opacity: ${braketsEnv === 'production' ? '1' : '0.55'};">
            <span class="gh-badge env-badge-production" style="font-size: 12px; padding: 0.25rem 0.65rem;">🛡️ PRODUCTION</span>
            <div style="font-size: 11px; color: var(--gh-text-muted); margin-top: 0.25rem;">Human-Gated Main</div>
          </div>
        </div>

        <!-- Live Checklist Container -->
        <div id="promotion-checklist-container">
          <div style="text-align: center; padding: 2rem; color: var(--gh-text-muted);">
            ⏳ Loading environment checklist and verification status...
          </div>
        </div>
      </div>

      <div style="padding: 1rem 1.25rem; border-top: 1px solid var(--gh-border); background: #0d1117; display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 0.5rem;">
        <button id="btn-rollback" class="gh-btn gh-btn-danger" onclick="rollbackEnvironment()" style="display: ${braketsEnv !== 'demo' ? 'inline-flex' : 'none'};">
          ⏪ Rollback
        </button>
        <div style="display: flex; gap: 0.5rem; margin-left: auto;">
          <button class="gh-btn" onclick="closePromotionModal()">Close</button>
          <button id="btn-request-promote" class="gh-btn" onclick="requestPromotion()" style="display: none;">
            📋 Request Promotion
          </button>
          <button id="btn-approve-promote" class="gh-btn gh-btn-primary" onclick="approvePromotion()" style="display: none;">
            ✍️ Supervisor Approve
          </button>
        </div>
      </div>
    </div>
  </div>

  <!-- Global Client Scripts -->
  <script>
    const currentProjectId = "${projectId}";

    function getAdminAuthHeaders() {
      const secret = localStorage.getItem("s3a_admin_secret");
      const headers = { "Content-Type": "application/json" };
      if (secret) {
        headers["Authorization"] = "Bearer " + secret;
        headers["X-Admin-Secret"] = secret;
      }
      return headers;
    }

    async function promptAndStoreAdminSecret() {
      const entered = prompt("Admin authorization required (Cloudflare Access or Bearer ADMIN_SECRET).\\nPlease enter ADMIN_SECRET:");
      if (entered && entered.trim()) {
        localStorage.setItem("s3a_admin_secret", entered.trim());
        return entered.trim();
      }
      return null;
    }

    async function openEnrollmentModal() {
      const modal = document.getElementById("enrollment-modal");
      if (!modal) return;
      modal.style.display = "flex";
      try {
        const { pubHex } = await getOrGenerateReviewerKey();
        const input = document.getElementById("enrollment-pubkey-input");
        if (input) input.value = pubHex;

        const badge = document.getElementById("enrollment-status-badge");
        if (badge) {
          badge.innerHTML = '<span class="gh-badge gh-badge-muted">Checking secret...</span>';
          try {
            const adminSecret = localStorage.getItem("s3a_admin_secret") || "";
            const headers = {};
            if (adminSecret) {
              headers["Authorization"] = "Bearer " + adminSecret;
              headers["X-Admin-Secret"] = adminSecret;
            }
            const res = await fetch("/admin/check-supervisor-key?key=" + pubHex, { headers });
            if (res.ok) {
              const data = await res.json();
              if (data.enrolled) {
                badge.innerHTML = '<span class="gh-badge gh-badge-success">✅ Authorized Supervisor Key</span>';
              } else {
                badge.innerHTML = '<span class="gh-badge gh-badge-danger">⚠️ Not Enrolled in SUPERVISOR_PUBKEYS (Approvals will return 403)</span>';
              }
            } else if (res.status === 401) {
              badge.innerHTML = '<span class="gh-badge gh-badge-yellow">🔒 Enter ADMIN_SECRET to verify status: <a href="javascript:void(0)" onclick="promptAndCheckKey()" style="color:var(--gh-link);">Authenticate</a></span>';
            } else {
              badge.innerHTML = '<span class="gh-badge gh-badge-muted">Status unavailable</span>';
            }
          } catch (e) {
            badge.innerHTML = '<span class="gh-badge gh-badge-muted">Status unavailable</span>';
          }
        }
      } catch (err) {
        alert("Key error: " + err.message);
      }
    }

    async function promptAndCheckKey() {
      const sec = await promptAndStoreAdminSecret();
      if (sec) openEnrollmentModal();
    }

    function closeEnrollmentModal() {
      const modal = document.getElementById("enrollment-modal");
      if (modal) modal.style.display = "none";
    }

    function copyEnrollmentPubkey() {
      const input = document.getElementById("enrollment-pubkey-input");
      if (!input) return;
      navigator.clipboard.writeText(input.value);
      alert("Supervisor public key copied to clipboard:\\n" + input.value);
    }

    async function toggleAutonomousEngine(action) {
      const btn = document.getElementById('engine-toggle-btn');
      if (btn) { btn.disabled = true; btn.textContent = '⏳ Updating...'; }
      try {
        const res = await fetch('/project/' + currentProjectId + '/control', {
          method: 'POST',
          headers: getAdminAuthHeaders(),
          body: JSON.stringify({ action })
        });
        if (res.ok) {
          location.reload();
        } else if (res.status === 401) {
          if (btn) { btn.disabled = false; btn.textContent = action === 'pause' ? '⏸ Pause Swarm' : '▶ Resume Swarm'; }
          const sec = await promptAndStoreAdminSecret();
          if (sec) toggleAutonomousEngine(action);
        } else {
          alert('Failed to update engine state: ' + await res.text());
          if (btn) { btn.disabled = false; btn.textContent = action === 'pause' ? '⏸ Pause Swarm' : '▶ Resume Swarm'; }
        }
      } catch (e) {
        alert('Control error: ' + e.message);
        if (btn) { btn.disabled = false; btn.textContent = action === 'pause' ? '⏸ Pause Swarm' : '▶ Resume Swarm'; }
      }
    }

    async function triggerManualTick() {
      const btn = document.getElementById('btn-manual-step');
      if (btn) { btn.disabled = true; btn.textContent = '⏳ Dispatching...'; }
      try {
        const res = await fetch('/project/' + currentProjectId + '/scheduler-tick', {
          method: 'POST',
          headers: getAdminAuthHeaders(),
        });
        if (res.status === 401) {
          if (btn) { btn.disabled = false; btn.textContent = '⚡ Run 1 Step'; }
          const sec = await promptAndStoreAdminSecret();
          if (sec) triggerManualTick();
          return;
        }
        const data = await res.json();
        alert('Autonomous Step Result: ' + (data.status || 'Dispatched') + (data.task_id ? ' (' + data.task_id + ')' : '') + (data.message ? ' - ' + data.message : ''));
        location.reload();
      } catch (e) {
        alert('Dispatch error: ' + e.message);
        if (btn) { btn.disabled = false; btn.textContent = '⚡ Run 1 Step'; }
      }
    }

    // WebCrypto Ed25519 Human Supervisor Key
    async function getOrGenerateReviewerKey() {
      const DB_NAME = "S3APMSupervisorDB";
      return new Promise((resolve, reject) => {
        const req = indexedDB.open(DB_NAME, 1);
        req.onupgradeneeded = (e) => {
          const db = e.target.result;
          if (!db.objectStoreNames.contains("keys")) db.createObjectStore("keys");
        };
        req.onsuccess = async (e) => {
          const db = e.target.result;
          const tx = db.transaction("keys", "readwrite");
          const store = tx.objectStore("keys");
          const getReq = store.get("pm_supervisor_ed25519");

          getReq.onsuccess = async () => {
            if (getReq.result) {
              resolve(getReq.result);
            } else {
              try {
                const kp = await window.crypto.subtle.generateKey(
                  { name: "Ed25519" },
                  false,
                  ["sign", "verify"]
                );
                const pubRaw = await window.crypto.subtle.exportKey("raw", kp.publicKey);
                const pubHex = Array.from(new Uint8Array(pubRaw)).map(b => b.toString(16).padStart(2, '0')).join('');
                const entry = { keyPair: kp, pubHex };
                const putTx = db.transaction("keys", "readwrite");
                putTx.objectStore("keys").put(entry, "pm_supervisor_ed25519");
                putTx.oncomplete = () => resolve(entry);
              } catch (genErr) {
                reject(genErr);
              }
            }
          };
          getReq.onerror = () => reject(getReq.error);
        };
        req.onerror = () => reject(req.error);
      });
    }

    // ------------------------------------------------------------------------
    // Environment Lifecycle & Promotion Modal Client Scripts
    // ------------------------------------------------------------------------
    let currentPromotionData = null;

    async function openPromotionModal() {
      const modal = document.getElementById("promotion-modal");
      if (!modal) return;
      modal.style.display = "flex";
      await fetchPromotionStatus();
    }

    function closePromotionModal() {
      const modal = document.getElementById("promotion-modal");
      if (modal) modal.style.display = "none";
    }

    async function fetchPromotionStatus() {
      const container = document.getElementById("promotion-checklist-container");
      if (!container) return;
      container.innerHTML = '<div style="text-align: center; padding: 2rem; color: var(--gh-text-muted);">⏳ Checking promotion checklist & verification status...</div>';
      try {
        const res = await fetch('/project/' + currentProjectId + '/env');
        if (!res.ok) {
          container.innerHTML = '<div style="color: var(--gh-red); padding: 1rem;">Failed to fetch promotion status: ' + await res.text() + '</div>';
          return;
        }
        const data = await res.json();
        currentPromotionData = data;
        renderPromotionChecklist(data);
      } catch (e) {
        container.innerHTML = '<div style="color: var(--gh-red); padding: 1rem;">Error: ' + e.message + '</div>';
      }
    }

    function renderPromotionChecklist(data) {
      const container = document.getElementById("promotion-checklist-container");
      const btnRequest = document.getElementById("btn-request-promote");
      const btnApprove = document.getElementById("btn-approve-promote");
      const btnRollback = document.getElementById("btn-rollback");
      if (!container) return;

      const currentEnv = data.brakets_env || "demo";
      const nextEnv = currentEnv === "demo" ? "test" : (currentEnv === "test" ? "production" : null);
      const activeChecklist = currentEnv === "demo" ? data.test_checklist : data.production_checklist;

      if (btnRollback) {
        btnRollback.style.display = currentEnv !== "demo" ? "inline-flex" : "none";
        btnRollback.textContent = currentEnv === "production" ? "⏪ Rollback to Test" : "⏪ Rollback to Demo";
      }

      if (!nextEnv) {
        container.innerHTML = '<div style="background: rgba(46, 160, 67, 0.15); border: 1px solid rgba(46, 160, 67, 0.4); border-radius: 6px; padding: 1.25rem; text-align: center;"><h4 style="color: #3fb950; margin-bottom: 0.5rem; font-size: 15px;">🛡️ Production Active</h4><p style="color: var(--gh-text); font-size: 13px; line-height: 1.5; margin: 0;">Brakets is running in <strong>Production</strong> under human gate enforcement (Ashley + Ward signed keys).<br><strong>Auto-merge is disabled</strong> for all production updates.</p></div>';
        if (btnRequest) btnRequest.style.display = "none";
        if (btnApprove) btnApprove.style.display = "none";
        return;
      }

      let html = '<div style="margin-bottom: 1rem;">' +
        '<div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.75rem;">' +
        '<div style="font-weight: 600; font-size: 14px; color: var(--gh-text-heading);">' +
        'Gate Checklist: Promoting <code>' + currentEnv.toUpperCase() + '</code> ➔ <code>' + nextEnv.toUpperCase() + '</code>' +
        '</div>' +
        '<span class="gh-badge ' + (activeChecklist && activeChecklist.ready ? 'gh-badge-success' : 'gh-badge-yellow') + '">' +
        (activeChecklist && activeChecklist.ready ? '✅ Requirements Met' : '⚠️ ' + ((activeChecklist && activeChecklist.blockers) ? activeChecklist.blockers.length : 0) + ' Blockers') +
        '</span></div>' +
        '<div style="display: flex; flex-direction: column; gap: 0.5rem;">';

      if (activeChecklist && activeChecklist.checklist) {
        for (const item of activeChecklist.checklist) {
          html += '<div style="background: #161b22; border: 1px solid ' + (item.passed ? 'rgba(46, 160, 67, 0.4)' : 'rgba(248, 81, 73, 0.4)') + '; border-radius: 6px; padding: 0.65rem 0.85rem; display: flex; align-items: flex-start; gap: 0.75rem;">' +
            '<span style="font-size: 15px;">' + (item.passed ? '✅' : '❌') + '</span>' +
            '<div style="flex: 1;">' +
            '<div style="font-size: 13px; font-weight: 600; color: ' + (item.passed ? 'var(--gh-text-heading)' : 'var(--gh-red)') + ';">' + item.title + '</div>' +
            '<div style="font-size: 12px; color: var(--gh-text-muted); margin-top: 0.2rem; font-family: monospace;">' + item.details + '</div>' +
            '</div></div>';
        }
      }

      html += '</div></div>';

      if (data.pending_promotion) {
        html += '<div style="background: rgba(56, 139, 253, 0.15); border: 1px solid rgba(56, 139, 253, 0.4); border-radius: 6px; padding: 0.85rem 1rem; margin-top: 1rem;">' +
          '<div style="font-weight: 600; color: #58a6ff; font-size: 13px;">📋 Promotion Pending Human Approval</div>' +
          '<div style="font-size: 12px; color: var(--gh-text); margin-top: 0.25rem;">' +
          'Requested by: <code>' + data.pending_promotion.requester_id + '</code> (' + data.pending_promotion.requester_type + ')<br>' +
          'Target: <strong>' + data.pending_promotion.target_env.toUpperCase() + '</strong>' +
          '</div></div>';
      }

      container.innerHTML = html;

      const isReady = activeChecklist && activeChecklist.ready;
      if (btnRequest) {
        btnRequest.style.display = "inline-flex";
        btnRequest.textContent = "📋 Request Promotion (" + nextEnv.toUpperCase() + ")";
        btnRequest.disabled = !isReady;
      }
      if (btnApprove) {
        btnApprove.style.display = "inline-flex";
        btnApprove.textContent = "✍️ Supervisor Approve (" + nextEnv.toUpperCase() + ")";
        btnApprove.disabled = !isReady;
      }
    }

    async function requestPromotion() {
      if (!currentPromotionData) return;
      const currentEnv = currentPromotionData.brakets_env || "demo";
      const targetEnv = currentEnv === "demo" ? "test" : "production";
      const rationale = prompt("Enter promotion request rationale:", "Rehearsal and verification checklist verified green.");
      if (rationale === null) return;

      const rollbackPlan = targetEnv === "production" 
        ? prompt("Enter production rollback plan:", "Immediate revert to previous deployment hash and switch brakets_env to test.") 
        : "Revert environment to demo.";
      if (rollbackPlan === null) return;

      try {
        const res = await fetch('/project/' + currentProjectId + '/env/request-promotion', {
          method: 'POST',
          headers: getAdminAuthHeaders(),
          body: JSON.stringify({
            target_env: targetEnv,
            requester_id: "antigravity",
            requester_type: "ai",
            rationale,
            rollback_plan: rollbackPlan
          })
        });
        const respData = await res.json();
        if (res.ok) {
          alert("Promotion requested successfully! Pending supervisor approval.");
          await fetchPromotionStatus();
        } else {
          alert("Failed to request promotion: " + (respData.error || JSON.stringify(respData)));
        }
      } catch (e) {
        alert("Request error: " + e.message);
      }
    }

    async function approvePromotion() {
      if (!currentPromotionData) return;
      const currentEnv = currentPromotionData.brakets_env || "demo";
      const targetEnv = currentEnv === "demo" ? "test" : "production";

      const rationale = prompt("Enter supervisor approval rationale (Ashley/Ward):", "Verified checklist and QA traces green. Approved promotion to " + targetEnv + ".");
      if (rationale === null) return;

      const rollbackPlan = targetEnv === "production"
        ? (prompt("Confirm rollback plan:", "Immediate revert commit and demote env to test.") || "Demote to test")
        : "Revert to demo";

      try {
        const { keyPair, pubHex } = await getOrGenerateReviewerKey();
        const tsSecs = Math.floor(Date.now() / 1000);
        const signPayload = new TextEncoder().encode("APPROVE_ENV_PROMOTION:" + targetEnv + ":" + tsSecs + ":" + rationale);
        const sigBuffer = await window.crypto.subtle.sign({ name: "Ed25519" }, keyPair.privateKey, signPayload);
        const sigHex = Array.from(new Uint8Array(sigBuffer)).map(b => b.toString(16).padStart(2, '0')).join('');

        const res = await fetch('/project/' + currentProjectId + '/env/approve-promotion', {
          method: 'POST',
          headers: getAdminAuthHeaders(),
          body: JSON.stringify({
            target_env: targetEnv,
            reviewer_pubkey: pubHex,
            signature: sigHex,
            rationale,
            rollback_plan: rollbackPlan,
            timestamp: tsSecs
          })
        });
        const respData = await res.json();
        if (res.ok) {
          alert("🎉 Environment promoted to " + targetEnv.toUpperCase() + "!");
          location.reload();
        } else {
          alert("Approval failed: " + (respData.error || JSON.stringify(respData)));
        }
      } catch (e) {
        alert("Approval error: " + e.message);
      }
    }

    async function rollbackEnvironment() {
      if (!confirm("Are you sure you want to rollback the environment?")) return;
      const rationale = prompt("Enter rollback rationale:", "Issues discovered in staging/production, reverting to previous stage.");
      if (rationale === null) return;

      try {
        const { keyPair, pubHex } = await getOrGenerateReviewerKey();
        const res = await fetch('/project/' + currentProjectId + '/env/rollback', {
          method: 'POST',
          headers: getAdminAuthHeaders(),
          body: JSON.stringify({
            reviewer_pubkey: pubHex,
            rationale
          })
        });
        const respData = await res.json();
        if (res.ok) {
          alert("Environment rolled back to " + respData.brakets_env.toUpperCase() + ".");
          location.reload();
        } else {
          alert("Rollback failed: " + (respData.error || JSON.stringify(respData)));
        }
      } catch (e) {
        alert("Rollback error: " + e.message);
      }
    }

    // --- Project Switcher & Project Creator Handlers ---

    function toggleProjectDropdown() {
      const menu = document.getElementById('project-dropdown-menu');
      if (menu) {
        menu.style.display = menu.style.display === 'block' ? 'none' : 'block';
      }
    }

    // Close dropdown on outside click
    document.addEventListener('click', function(e) {
      const switcherBtn = document.getElementById('project-switcher-btn');
      const menu = document.getElementById('project-dropdown-menu');
      if (menu && switcherBtn && !switcherBtn.contains(e.target) && !menu.contains(e.target)) {
        menu.style.display = 'none';
      }
    });

    function openCreateProjectModal() {
      const modal = document.getElementById('create-project-modal');
      if (modal) {
        modal.style.display = 'flex';
        const input = document.getElementById('create-project-prompt');
        if (input) input.focus();
      }
      const menu = document.getElementById('project-dropdown-menu');
      if (menu) menu.style.display = 'none';
    }

    function closeCreateProjectModal() {
      const modal = document.getElementById('create-project-modal');
      if (modal) modal.style.display = 'none';
      const statusDiv = document.getElementById('create-project-status');
      if (statusDiv) statusDiv.style.display = 'none';
    }

    function selectPresetPrompt(promptText, nameText, idText, templateKey) {
      const promptEl = document.getElementById('create-project-prompt');
      const nameEl = document.getElementById('create-project-name');
      const idEl = document.getElementById('create-project-id');
      const tmplEl = document.getElementById('create-project-template');
      if (promptEl) promptEl.value = promptText;
      if (nameEl) nameEl.value = nameText;
      if (idEl) idEl.value = idText;
      if (tmplEl) tmplEl.value = templateKey;
    }

    async function createProjectDirect(promptText, nameText, idText, templateKey) {
      selectPresetPrompt(promptText, nameText, idText, templateKey);
      openCreateProjectModal();
      await submitCreateProject();
    }

    async function submitCreateProject() {
      const promptEl = document.getElementById('create-project-prompt');
      const nameEl = document.getElementById('create-project-name');
      const idEl = document.getElementById('create-project-id');
      const tmplEl = document.getElementById('create-project-template');
      const statusDiv = document.getElementById('create-project-status');
      const submitBtn = document.getElementById('btn-submit-create-project');

      const promptVal = promptEl ? promptEl.value.trim() : "";
      if (!promptVal) {
        alert("Please enter a project prompt or select a template.");
        return;
      }

      const nameVal = nameEl ? nameEl.value.trim() : "";
      const idVal = idEl ? idEl.value.trim() : "";
      const tmplVal = tmplEl ? tmplEl.value.trim() : "";

      if (statusDiv) {
        statusDiv.style.display = 'block';
        statusDiv.style.background = 'rgba(56, 139, 253, 0.15)';
        statusDiv.style.color = '#58a6ff';
        statusDiv.innerText = "⏳ Initializing project, isolated repository, issue board & format packs...";
      }
      if (submitBtn) submitBtn.disabled = true;

      try {
        const res = await fetch('/api/projects', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            prompt: promptVal,
            name: nameVal || undefined,
            id: idVal || undefined,
            template: tmplVal || undefined
          })
        });

        const data = await res.json();
        if (res.ok && data.success) {
          if (statusDiv) {
            statusDiv.style.background = 'rgba(46, 160, 67, 0.2)';
            statusDiv.style.color = '#3fb950';
            statusDiv.innerText = "✓ Project created! Redirecting to workspace...";
          }
          setTimeout(() => {
            window.location.href = data.project?.url || ('/p/' + data.project?.id + '/code');
          }, 600);
        } else {
          if (statusDiv) {
            statusDiv.style.background = 'rgba(218, 54, 51, 0.15)';
            statusDiv.style.color = '#f85149';
            statusDiv.innerText = "Error: " + (data.error || "Failed to create project");
          }
          if (submitBtn) submitBtn.disabled = false;
        }
      } catch (e) {
        if (statusDiv) {
          statusDiv.style.background = 'rgba(218, 54, 51, 0.15)';
          statusDiv.style.color = '#f85149';
          statusDiv.innerText = "Error: " + e.message;
        }
        if (submitBtn) submitBtn.disabled = false;
      }
    }
  </script>

  <!-- Modal: Create Project -->
  <div id="create-project-modal" class="modal-overlay" onclick="if(event.target === this) closeCreateProjectModal()">
    <div style="background: var(--gh-card-bg); border: 1px solid var(--gh-border); border-radius: 8px; width: 100%; max-width: 620px; box-shadow: 0 16px 36px rgba(0,0,0,0.7); overflow: hidden;">
      <div style="padding: 1rem 1.25rem; border-bottom: 1px solid var(--gh-border); display: flex; justify-content: space-between; align-items: center; background: #0d1117;">
        <div style="display: flex; align-items: center; gap: 0.6rem;">
          <span style="font-size: 1.2rem;">🚀</span>
          <strong style="color: #f0f6fc; font-size: 1.1rem;">Create Project in Swarm</strong>
        </div>
        <button onclick="closeCreateProjectModal()" style="background: transparent; border: none; color: var(--gh-text-muted); font-size: 1.2rem; cursor: pointer;">✕</button>
      </div>
      <div style="padding: 1.25rem;">
        <p style="color: var(--gh-text-muted); font-size: 13px; margin-bottom: 1rem;">
          Tell Swarm what you want to build. The platform will initialize an isolated project workspace, repository, seeded issue board, and format/skill packs.
        </p>

        <!-- Quick Preset Cards -->
        <label style="display: block; font-weight: 600; font-size: 12px; color: var(--gh-text-heading); margin-bottom: 0.5rem; text-transform: uppercase;">
          Quick Starter Templates
        </label>
        <div style="display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 0.6rem; margin-bottom: 1.25rem;">
          <button type="button" class="preset-card" onclick="selectPresetPrompt('make me a micro-SaaS', 'InvoiceFlow Micro-SaaS', 'micro-saas', 'micro_saas')" style="background: #0d1117; border: 1px solid var(--gh-border); border-radius: 6px; padding: 0.75rem 0.6rem; text-align: left; cursor: pointer; color: var(--gh-text); transition: border-color 0.15s;">
            <div style="font-size: 1.1rem; margin-bottom: 0.25rem;">⚡</div>
            <strong style="font-size: 12px; color: #f0f6fc; display: block; margin-bottom: 0.2rem;">Micro-SaaS</strong>
            <span style="font-size: 11px; color: var(--gh-text-muted); display: block; line-height: 1.3;">Stripe billing, D1/R2, Next.js 15, Auth</span>
          </button>
          <button type="button" class="preset-card" onclick="selectPresetPrompt('fix this repo', 'Repo Auto-Healer', 'repo-fixer', 'fix_repo')" style="background: #0d1117; border: 1px solid var(--gh-border); border-radius: 6px; padding: 0.75rem 0.6rem; text-align: left; cursor: pointer; color: var(--gh-text); transition: border-color 0.15s;">
            <div style="font-size: 1.1rem; margin-bottom: 0.25rem;">🛠️</div>
            <strong style="font-size: 12px; color: #f0f6fc; display: block; margin-bottom: 0.2rem;">Fix This Repo</strong>
            <span style="font-size: 11px; color: var(--gh-text-muted); display: block; line-height: 1.3;">Bug triage, AST audit, regression suite</span>
          </button>
          <button type="button" class="preset-card" onclick="selectPresetPrompt('create a three.js 3D site', 'Spatial 3D Canvas', 'threejs-3d-site', 'threejs_site')" style="background: #0d1117; border: 1px solid var(--gh-border); border-radius: 6px; padding: 0.75rem 0.6rem; text-align: left; cursor: pointer; color: var(--gh-text); transition: border-color 0.15s;">
            <div style="font-size: 1.1rem; margin-bottom: 0.25rem;">🎨</div>
            <strong style="font-size: 12px; color: #f0f6fc; display: block; margin-bottom: 0.2rem;">Three.js 3D Site</strong>
            <span style="font-size: 11px; color: var(--gh-text-muted); display: block; line-height: 1.3;">WebGL 3D canvas, OrbitControls, Shaders</span>
          </button>
        </div>

        <!-- Custom Prompt & Form -->
        <div style="margin-bottom: 1rem;">
          <label for="create-project-prompt" style="display: block; font-weight: 600; font-size: 12.5px; color: var(--gh-text-heading); margin-bottom: 0.35rem;">
            Project Goal / Prompt:
          </label>
          <textarea id="create-project-prompt" rows="3" placeholder="e.g. make me a micro-SaaS with Stripe or fix this repo" style="width: 100%; background: #0d1117; border: 1px solid var(--gh-border); border-radius: 6px; color: #f0f6fc; font-family: inherit; font-size: 13px; padding: 0.6rem 0.8rem; line-height: 1.4; resize: vertical;"></textarea>
        </div>

        <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 0.75rem; margin-bottom: 1.25rem;">
          <div>
            <label for="create-project-name" style="display: block; font-weight: 600; font-size: 12px; color: var(--gh-text-heading); margin-bottom: 0.35rem;">
              Project Name:
            </label>
            <input type="text" id="create-project-name" placeholder="InvoiceFlow Micro-SaaS" style="width: 100%; background: #0d1117; border: 1px solid var(--gh-border); border-radius: 6px; color: #f0f6fc; font-size: 13px; padding: 0.5rem 0.75rem;">
          </div>
          <div>
            <label for="create-project-id" style="display: block; font-weight: 600; font-size: 12px; color: var(--gh-text-heading); margin-bottom: 0.35rem;">
              Project Slug / ID:
            </label>
            <input type="text" id="create-project-id" placeholder="micro-saas" style="width: 100%; background: #0d1117; border: 1px solid var(--gh-border); border-radius: 6px; color: #7ee787; font-family: monospace; font-size: 12.5px; padding: 0.5rem 0.75rem;">
          </div>
        </div>

        <input type="hidden" id="create-project-template" value="">

        <div id="create-project-status" style="display: none; font-size: 12.5px; padding: 0.6rem 0.8rem; border-radius: 6px; margin-bottom: 1rem;"></div>

        <div style="display: flex; justify-content: flex-end; gap: 0.5rem;">
          <button type="button" class="gh-btn" onclick="closeCreateProjectModal()">Cancel</button>
          <button type="button" id="btn-submit-create-project" class="gh-btn gh-btn-primary" onclick="submitCreateProject()">
            🚀 Create Project
          </button>
        </div>
      </div>
    </div>
  </div>
</body>
</html>`;
}
