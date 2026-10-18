// cloudflare/src/ui/pages/platform.ts
// Swarm AI Agentic GitHub Platform Pages
// Platform UI: Overview, Projects Directory & Creator, Swarm Agents, Compute Ledger, System Traces

import type { ProjectRegistryItem, RegisteredAgent, BudgetStatus } from "../../types";

export interface PlatformPageOptions {
  section: "overview" | "projects" | "agents" | "ledger" | "traces";
  projects: ProjectRegistryItem[];
  agents?: RegisteredAgent[];
  budget?: BudgetStatus;
  traces?: any[];
}

export function renderPlatformPage(options: PlatformPageOptions): string {
  const { section, projects = [], agents = [], budget, traces = [] } = options;

  switch (section) {
    case "projects":
      return renderProjectsSection(projects);
    case "agents":
      return renderAgentsSection(agents);
    case "ledger":
      return renderLedgerSection(budget);
    case "traces":
      return renderTracesSection(traces);
    case "overview":
    default:
      return renderOverviewSection(projects, agents, budget);
  }
}

function renderOverviewSection(projects: ProjectRegistryItem[], agents: RegisteredAgent[], budget?: BudgetStatus): string {
  const spentToday = budget?.spent_today ?? 0.01;
  const cap = budget?.cap ?? 5.0;
  const pctSpent = cap > 0 ? (spentToday / cap) * 100 : 0;

  return `
    <div style="display: flex; flex-direction: column; gap: 1.5rem;">
      <!-- Hero Banner -->
      <div class="gh-card" style="background: linear-gradient(135deg, #161b22 0%, #0d1117 100%); border-left: 4px solid var(--gh-orange);">
        <div class="gh-card-body" style="padding: 1.5rem;">
          <div style="display: flex; justify-content: space-between; align-items: flex-start; flex-wrap: wrap; gap: 1rem;">
            <div>
              <div style="display: flex; align-items: center; gap: 0.6rem; margin-bottom: 0.4rem;">
                <span style="color: var(--gh-orange); font-size: 1.5rem; font-weight: 800; letter-spacing: -0.5px;">swarm</span>
                <span class="gh-badge gh-badge-purple" style="font-weight: 700;">AI Agentic GitHub Platform</span>
                <span class="gh-badge gh-badge-success">Online</span>
              </div>
              <p style="color: var(--gh-text-muted); font-size: 13.5px; max-width: 720px; line-height: 1.5; margin: 0;">
                Autonomous multi-agent operating system coordinating PM, Builder, QA, Budget, Librarian, and Scrum agents.
                Every project is fully isolated with its own repository, issue board, format/skill packs, and dual human supervisor gating.
              </p>
            </div>
            <div style="display: flex; gap: 0.6rem; align-items: center;">
              <button onclick="openCreateProjectModal()" class="gh-btn gh-btn-primary" style="font-weight: 600;">
                + Create Project
              </button>
              <a href="/p/brakets/code" class="gh-btn">
                Open Brakets ↗
              </a>
            </div>
          </div>
        </div>
      </div>

      <!-- Quick Starter Presets Bar -->
      <div class="gh-card">
        <div class="gh-card-header" style="display: flex; justify-content: space-between; align-items: center;">
          <div style="font-weight: 600; color: var(--gh-text-heading); font-size: 13.5px;">
            ⚡ Create a Project with Swarm
          </div>
          <span style="font-size: 11.5px; color: var(--gh-text-muted);">One-click setup for isolated repo, issue board & skill packs</span>
        </div>
        <div class="gh-card-body" style="padding: 1.25rem;">
          <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 1rem;">
            
            <!-- Preset 1: Micro-SaaS -->
            <div style="background: #0d1117; border: 1px solid var(--gh-border); border-radius: 6px; padding: 1rem; display: flex; flex-direction: column; justify-content: space-between;">
              <div>
                <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.4rem;">
                  <span style="font-size: 1.3rem;">⚡</span>
                  <span class="gh-badge gh-badge-blue">micro_saas</span>
                </div>
                <strong style="color: var(--gh-text-heading); font-size: 13.5px; display: block; margin-bottom: 0.3rem;">
                  "make me a micro-SaaS"
                </strong>
                <p style="color: var(--gh-text-muted); font-size: 12px; line-height: 1.4; margin-bottom: 0.8rem;">
                  Next.js 15, Stripe billing, Cloudflare D1/R2, Lucia Auth, and subscriber checkout integration.
                </p>
              </div>
              <button onclick="createProjectDirect('make me a micro-SaaS', 'InvoiceFlow Micro-SaaS', 'micro-saas', 'micro_saas')" class="gh-btn gh-btn-sm gh-btn-primary" style="width: 100%; justify-content: center;">
                Launch Micro-SaaS Project
              </button>
            </div>

            <!-- Preset 2: Fix Repo -->
            <div style="background: #0d1117; border: 1px solid var(--gh-border); border-radius: 6px; padding: 1rem; display: flex; flex-direction: column; justify-content: space-between;">
              <div>
                <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.4rem;">
                  <span style="font-size: 1.3rem;">🛠️</span>
                  <span class="gh-badge gh-badge-yellow">fix_repo</span>
                </div>
                <strong style="color: var(--gh-text-heading); font-size: 13.5px; display: block; margin-bottom: 0.3rem;">
                  "fix this repo"
                </strong>
                <p style="color: var(--gh-text-muted); font-size: 12px; line-height: 1.4; margin-bottom: 0.8rem;">
                  Static analysis, type error remediation, dead code pruning, and automated regression test suite.
                </p>
              </div>
              <button onclick="createProjectDirect('fix this repo', 'Repo Auto-Healer', 'repo-fixer', 'fix_repo')" class="gh-btn gh-btn-sm gh-btn-primary" style="width: 100%; justify-content: center;">
                Launch Repo Healer Project
              </button>
            </div>

            <!-- Preset 3: Three.js 3D Site -->
            <div style="background: #0d1117; border: 1px solid var(--gh-border); border-radius: 6px; padding: 1rem; display: flex; flex-direction: column; justify-content: space-between;">
              <div>
                <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.4rem;">
                  <span style="font-size: 1.3rem;">🎨</span>
                  <span class="gh-badge gh-badge-purple">threejs_site</span>
                </div>
                <strong style="color: var(--gh-text-heading); font-size: 13.5px; display: block; margin-bottom: 0.3rem;">
                  "create a three.js 3D site"
                </strong>
                <p style="color: var(--gh-text-muted); font-size: 12px; line-height: 1.4; margin-bottom: 0.8rem;">
                  Interactive WebGL 3D canvas, GLTF model loader, PBR lighting, camera OrbitControls, and GLSL shaders.
                </p>
              </div>
              <button onclick="createProjectDirect('create a three.js 3D site', 'Spatial 3D Canvas', 'threejs-3d-site', 'threejs_site')" class="gh-btn gh-btn-sm gh-btn-primary" style="width: 100%; justify-content: center;">
                Launch Three.js Project
              </button>
            </div>

          </div>
        </div>
      </div>

      <!-- Platform Active Projects & Metrics -->
      <div style="display: grid; grid-template-columns: 2fr 1fr; gap: 1.5rem;">
        
        <!-- Active Projects List -->
        <div class="gh-card">
          <div class="gh-card-header" style="display: flex; justify-content: space-between; align-items: center;">
            <div style="font-weight: 600; color: var(--gh-text-heading); font-size: 13.5px;">
              📁 Active Projects (${projects.length})
            </div>
            <a href="/platform/projects" style="font-size: 12px;">View all projects &rarr;</a>
          </div>
          <div style="display: flex; flex-direction: column;">
            ${projects.map(proj => {
              const envClass = proj.env === 'production' ? 'env-badge-production' : (proj.env === 'test' ? 'env-badge-test' : 'env-badge-demo');
              return `
                <div class="gh-row" style="padding: 1rem 1.25rem;">
                  <div style="flex: 1;">
                    <div style="display: flex; align-items: center; gap: 0.5rem; margin-bottom: 0.25rem;">
                      <a href="/p/${proj.id}/code" style="font-weight: 600; font-size: 14px; color: var(--gh-link);">
                        ${proj.name}
                      </a>
                      <span style="font-size: 12px; color: var(--gh-text-muted);">(${proj.slug})</span>
                      <span class="repo-pill env-badge ${envClass}" style="font-size: 10px; padding: 0.1rem 0.4rem;">
                        ENV: ${proj.env.toUpperCase()}
                      </span>
                    </div>
                    <div style="font-size: 12px; color: var(--gh-text-muted); margin-bottom: 0.4rem;">
                      ${proj.description}
                    </div>
                    <div style="font-size: 11.5px; color: var(--gh-text-muted); display: flex; gap: 0.8rem; align-items: center;">
                      <span>🤖 Agent: <strong style="color: #58a6ff;">${proj.active_agent || 'gate-agent'}</strong></span>
                      <span>&bull;</span>
                      <span>📋 Working on: <strong style="color: var(--gh-text-heading);">${proj.active_task || 'Backlog Triage'}</strong></span>
                    </div>
                  </div>
                  <div style="display: flex; gap: 0.4rem;">
                    <a href="/p/${proj.id}/code" class="gh-btn gh-btn-sm">Code</a>
                    <a href="/p/${proj.id}/issues" class="gh-btn gh-btn-sm">Issues</a>
                    <a href="/p/${proj.id}/pulls" class="gh-btn gh-btn-sm">Pulls</a>
                  </div>
                </div>
              `;
            }).join('')}
          </div>
        </div>

        <!-- Platform Health & Budget Column -->
        <div style="display: flex; flex-direction: column; gap: 1rem;">
          <div class="gh-card">
            <div class="gh-card-header">
              <strong style="color: var(--gh-text-heading); font-size: 13px;">💰 Swarm Compute Ledger</strong>
            </div>
            <div class="gh-card-body" style="font-size: 13px;">
              <div style="display: flex; justify-content: space-between; margin-bottom: 0.5rem;">
                <span style="color: var(--gh-text-muted);">Daily Spend:</span>
                <strong style="color: var(--gh-text-heading);">$${spentToday.toFixed(3)} / $${cap.toFixed(2)}</strong>
              </div>
              <div style="background: #21262d; border-radius: 4px; height: 8px; overflow: hidden; margin-bottom: 0.8rem;">
                <div style="background: var(--gh-green); height: 100%; width: ${Math.min(pctSpent, 100)}%;"></div>
              </div>
              <div style="font-size: 11.5px; color: var(--gh-text-muted); line-height: 1.4;">
                Metered compute across Workers AI, Cloudflare Sandbox, Headless Browser QA, and R2 Evidence.
              </div>
              <div style="margin-top: 0.8rem; border-top: 1px solid var(--gh-border); padding-top: 0.8rem;">
                <a href="/platform/ledger" style="font-size: 12px;">View Ledger Breakdown &rarr;</a>
              </div>
            </div>
          </div>

          <div class="gh-card">
            <div class="gh-card-header">
              <strong style="color: var(--gh-text-heading); font-size: 13px;">🛡️ Governance & Supervison</strong>
            </div>
            <div class="gh-card-body" style="font-size: 12.5px; color: var(--gh-text-muted); line-height: 1.5;">
              <div style="margin-bottom: 0.5rem;">
                <strong style="color: var(--gh-text-heading);">Ontology Rule:</strong> <code>:HumanActor owl:disjointWith :AIAgent</code>
              </div>
              <p style="margin-bottom: 0.5rem;">
                AI agents draft plans and code; only Ashley + Ward (SUPERVISOR_PUBKEYS) approve plans, merge CRs, or promote environments.
              </p>
              <div style="display: flex; gap: 0.4rem; margin-top: 0.6rem;">
                <a href="/platform/agents" class="gh-btn gh-btn-sm" style="flex: 1; justify-content: center;">Agents</a>
                <a href="/platform/traces" class="gh-btn gh-btn-sm" style="flex: 1; justify-content: center;">Traces</a>
              </div>
            </div>
          </div>
        </div>

      </div>
    </div>
  `;
}

function renderProjectsSection(projects: ProjectRegistryItem[]): string {
  return `
    <div style="display: flex; flex-direction: column; gap: 1.5rem;">
      <div style="display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 1rem;">
        <div>
          <h2 style="font-size: 1.3rem; font-weight: 700; color: var(--gh-text-heading); margin-bottom: 0.2rem;">
            Swarm Projects Directory
          </h2>
          <p style="font-size: 13px; color: var(--gh-text-muted); margin: 0;">
            Every project runs in an isolated Durable Object coordinator with its own git repository, issue board, and format packs.
          </p>
        </div>
        <button onclick="openCreateProjectModal()" class="gh-btn gh-btn-primary">
          + Create New Project
        </button>
      </div>

      <!-- Projects Grid -->
      <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(320px, 1fr)); gap: 1rem;">
        ${projects.map(p => {
          const envClass = p.env === 'production' ? 'env-badge-production' : (p.env === 'test' ? 'env-badge-test' : 'env-badge-demo');
          return `
            <div class="gh-card" style="display: flex; flex-direction: column; justify-content: space-between;">
              <div class="gh-card-body" style="padding: 1.25rem;">
                <div style="display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 0.5rem;">
                  <strong style="font-size: 15px; color: var(--gh-text-heading);">
                    ${p.name}
                  </strong>
                  <span class="repo-pill env-badge ${envClass}" style="font-size: 10px; padding: 0.15rem 0.45rem;">
                    ENV: ${p.env.toUpperCase()}
                  </span>
                </div>
                <div style="font-size: 12px; color: var(--gh-link); font-family: monospace; margin-bottom: 0.6rem;">
                  /p/${p.id} &bull; ${p.slug}
                </div>
                <p style="font-size: 12.5px; color: var(--gh-text-muted); line-height: 1.4; margin-bottom: 1rem;">
                  ${p.description}
                </p>
                <div style="background: #0d1117; border: 1px solid var(--gh-border); border-radius: 4px; padding: 0.6rem 0.8rem; font-size: 12px; margin-bottom: 1rem;">
                  <div style="color: var(--gh-text-muted); margin-bottom: 0.2rem;">Active Agent: <strong style="color: #58a6ff;">${p.active_agent || 'gate-agent'}</strong></div>
                  <div style="color: var(--gh-text-muted);">Current Task: <strong style="color: var(--gh-text-heading);">${p.active_task || 'Backlog Triage'}</strong></div>
                </div>
              </div>
              <div class="gh-card-footer" style="padding: 0.8rem 1.25rem; background: #0d1117; border-top: 1px solid var(--gh-border); display: flex; gap: 0.4rem; justify-content: flex-end;">
                <a href="/p/${p.id}/code" class="gh-btn gh-btn-sm gh-btn-primary">Open Code</a>
                <a href="/p/${p.id}/issues" class="gh-btn gh-btn-sm">Issues</a>
                <a href="/p/${p.id}/pulls" class="gh-btn gh-btn-sm">Pulls</a>
              </div>
            </div>
          `;
        }).join('')}
      </div>
    </div>
  `;
}

function renderAgentsSection(agents: RegisteredAgent[]): string {
  return `
    <div style="display: flex; flex-direction: column; gap: 1.5rem;">
      <div>
        <h2 style="font-size: 1.3rem; font-weight: 700; color: var(--gh-text-heading); margin-bottom: 0.2rem;">
          Swarm Autonomous Agents
        </h2>
        <p style="font-size: 13px; color: var(--gh-text-muted); margin: 0;">
          All registered agents operating on the Swarm platform with Ed25519 cryptographic keys and role envelopes.
        </p>
      </div>

      <div class="gh-card">
        <div class="gh-card-header">
          <strong style="color: var(--gh-text-heading); font-size: 13px;">Active Swarm Agents (${agents.length})</strong>
        </div>
        <div style="display: flex; flex-direction: column;">
          ${(agents.length > 0 ? agents : [
            {
              agent_id: "gate-agent",
              agent_name: "Gate Agent (Autonomous PM/Builder)",
              agent_role: "builder",
              public_key_hex: "d4e2a8...3b1c",
              registered_at: Date.now() - 86400000,
              capabilities: ["plan", "code_patch", "test_sandbox", "browser_qa"]
            },
            {
              agent_id: "antigravity",
              agent_name: "Antigravity Assistant",
              agent_role: "pm",
              public_key_hex: "8f1671...0d82",
              registered_at: Date.now() - 172800000,
              capabilities: ["triage", "plan", "evidence_audit"]
            }
          ]).map(a => `
            <div class="gh-row" style="padding: 1rem 1.25rem;">
              <div>
                <div style="display: flex; align-items: center; gap: 0.5rem; margin-bottom: 0.25rem;">
                  <strong style="color: var(--gh-text-heading); font-size: 14px;">${a.agent_name || a.agent_id}</strong>
                  <span class="gh-badge gh-badge-purple">${a.agent_role || 'agent'}</span>
                  <span class="gh-badge gh-badge-blue">Ed25519</span>
                </div>
                <div style="font-size: 12px; color: var(--gh-text-muted); font-family: monospace;">
                  ID: ${a.agent_id} &bull; Key: ${(a.public_key_hex || '').slice(0, 20)}...
                </div>
              </div>
              <div>
                <span class="gh-badge gh-badge-success">Registered</span>
              </div>
            </div>
          `).join('')}
        </div>
      </div>
    </div>
  `;
}

function renderLedgerSection(budget?: BudgetStatus): string {
  const spent = budget?.spent_today ?? 0.01;
  const cap = budget?.cap ?? 5.0;
  const entries = budget?.entries || [];

  return `
    <div style="display: flex; flex-direction: column; gap: 1.5rem;">
      <div>
        <h2 style="font-size: 1.3rem; font-weight: 700; color: var(--gh-text-heading); margin-bottom: 0.2rem;">
          Swarm Global Compute Ledger
        </h2>
        <p style="font-size: 13px; color: var(--gh-text-muted); margin: 0;">
          All compute, token, and storage expenditures are recorded in the immutable Swarm ledger.
        </p>
      </div>

      <div class="gh-card">
        <div class="gh-card-header" style="display: flex; justify-content: space-between;">
          <strong style="color: var(--gh-text-heading); font-size: 13.5px;">Budget Status</strong>
          <span style="font-size: 12px; color: var(--gh-text-muted);">Daily Cap: $${cap.toFixed(2)} (Resets 00:00 UTC)</span>
        </div>
        <div class="gh-card-body" style="padding: 1.25rem;">
          <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 1rem; margin-bottom: 1rem;">
            <div style="background: #0d1117; border: 1px solid var(--gh-border); padding: 0.8rem; border-radius: 6px;">
              <span style="font-size: 11px; color: var(--gh-text-muted); text-transform: uppercase;">Spent Today</span>
              <div style="font-size: 1.4rem; font-weight: 700; color: var(--gh-text-heading);">$${spent.toFixed(4)}</div>
            </div>
            <div style="background: #0d1117; border: 1px solid var(--gh-border); padding: 0.8rem; border-radius: 6px;">
              <span style="font-size: 11px; color: var(--gh-text-muted); text-transform: uppercase;">Remaining Cap</span>
              <div style="font-size: 1.4rem; font-weight: 700; color: #3fb950;">$${Math.max(0, cap - spent).toFixed(4)}</div>
            </div>
            <div style="background: #0d1117; border: 1px solid var(--gh-border); padding: 0.8rem; border-radius: 6px;">
              <span style="font-size: 11px; color: var(--gh-text-muted); text-transform: uppercase;">Triage Mode</span>
              <div style="font-size: 1.4rem; font-weight: 700; color: #58a6ff;">Active (Uncapped)</div>
            </div>
          </div>
        </div>
      </div>
    </div>
  `;
}

function renderTracesSection(traces: any[]): string {
  return `
    <div style="display: flex; flex-direction: column; gap: 1.5rem;">
      <div>
        <h2 style="font-size: 1.3rem; font-weight: 700; color: var(--gh-text-heading); margin-bottom: 0.2rem;">
          Swarm System Traces & Activity Stream
        </h2>
        <p style="font-size: 13px; color: var(--gh-text-muted); margin: 0;">
          W3C xAPI activity stream events grounded in the S3A ontology with actor provenance and JSONL export.
        </p>
      </div>

      <div class="gh-card">
        <div class="gh-card-header">
          <strong style="color: var(--gh-text-heading); font-size: 13.5px;">Recent xAPI Traces</strong>
        </div>
        <div class="gh-card-body" style="padding: 1.25rem;">
          <p style="color: var(--gh-text-muted); font-size: 13px;">
            Traces across all projects are validated against per-role xAPI profiles before write. Non-conforming actions are strictly rejected.
          </p>
          <div style="margin-top: 1rem;">
            <a href="/project/brakets/trace/TASK-283" class="gh-btn gh-btn-sm">View Brakets TASK-283 Trace ↗</a>
          </div>
        </div>
      </div>
    </div>
  `;
}
