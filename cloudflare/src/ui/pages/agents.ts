// cloudflare/src/ui/pages/agents.ts
// Registered Agents Page (/p/:id/agents)

import type { RegisteredAgent } from "../../types";

export interface AgentsPageOptions {
  projectId: string;
  registeredAgents?: RegisteredAgent[];
}

export function renderAgentsPage(options: AgentsPageOptions): string {
  const { projectId, registeredAgents = [] } = options;

  function getAgentRoleName(agentId?: string): string {
    if (!agentId || typeof agentId !== "string") return "Autonomous Agent";
    if (agentId.includes("builder") || agentId.includes("auto")) return "Autonomous Builder";
    if (agentId.includes("escrow")) return "Escrow Guard";
    if (agentId.includes("neon") || agentId.includes("token")) return "Branding Agent";
    if (agentId.includes("chaos")) return "QA Chaos Fuzzer";
    if (agentId.includes("journey")) return "QA Journey Runner";
    if (agentId.includes("visual")) return "QA Visual Inspector";
    return "Autonomous Agent";
  }

  return `
    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1rem; gap: 1rem; flex-wrap: wrap;">
      <div>
        <h2 style="font-size: 1.25rem; font-weight: 600; color: var(--gh-text-heading); margin-bottom: 0.2rem;">
          Registered Autonomous Agents
        </h2>
        <p style="color: var(--gh-text-muted); font-size: 13px;">
          Live registered agents executing in Cloudflare Sandboxes &amp; Workflows.
        </p>
      </div>
      <div>
        <span class="gh-badge gh-badge-blue">${registeredAgents.length} Agents Registered</span>
      </div>
    </div>

    <div class="gh-card">
      <div class="gh-card-header" style="background: #161b22;">
        <span style="font-weight: 600; color: var(--gh-text-heading); font-size: 13px;">Agent Roster</span>
        <span style="font-size: 12px; color: var(--gh-text-muted);">Ed25519 Verified Signatures</span>
      </div>

      <div style="display: flex; flex-direction: column;">
        ${registeredAgents.length === 0
          ? `
            <div class="gh-blankslate">
              <div class="gh-blankslate-icon">🤖</div>
              <h3>No agents currently registered</h3>
              <p>The swarm is currently idle. When task events are dispatched, agents self-register and begin working.</p>
              <button class="gh-btn" onclick="triggerManualTick()">
                ⚡ Run 1 Step to Dispatch Agent
              </button>
            </div>
          `
          : registeredAgents.map(a => {
            const agentId = a.agent_id || (a as any).agentId || "agent";
            const roleName = a.agent_role ? a.agent_role.toUpperCase() : getAgentRoleName(agentId);
            const status = a.status || "idle";
            const statusColor = status === 'working' ? 'gh-badge-blue' :
                                status === 'merged' ? 'gh-badge-purple' :
                                status === 'rejected' ? 'gh-badge-danger' :
                                status === 'qa_running' ? 'gh-badge-yellow' : 'gh-badge-muted';
            const pubkey = a.agent_pubkey || (a as any).pubkey || "";
            const taskId = a.task_id || (a as any).taskId || (a as any).task;
            const commits = a.commit_count ?? (a as any).commits ?? 0;
            const lastSeen = a.last_seen || (a as any).lastTs;

            return `
              <div class="gh-row" style="align-items: center; gap: 1rem; flex-wrap: wrap;">
                <div style="display: flex; align-items: center; gap: 0.75rem;">
                  <div style="width: 36px; height: 36px; border-radius: 50%; background: #21262d; border: 1px solid var(--gh-border); display: flex; align-items: center; justify-content: center; font-size: 1.1rem;">
                    🤖
                  </div>
                  <div>
                    <div style="font-weight: 600; color: var(--gh-text-heading); font-size: 14px;">
                      <code>${agentId}</code>
                    </div>
                    <div style="font-size: 12px; color: var(--gh-text-muted);">
                      Role: <strong>${roleName}</strong> &bull; Pubkey: <code>${pubkey.slice(0, 14)}...</code>
                      ${(a as any).files_allowed && (a as any).files_allowed.length ? `&bull; Scopes: <code style="font-size: 11px; color: var(--gh-text);">${(a as any).files_allowed.join(", ")}</code>` : ""}
                    </div>
                  </div>
                </div>

                <div style="display: flex; gap: 1rem; align-items: center; margin-left: auto; flex-wrap: wrap;">
                  <div>
                    <span class="gh-badge ${statusColor}">${status}</span>
                  </div>
                  ${taskId ? `
                    <div style="font-size: 12px;">
                      Task: <a href="/p/${projectId}/issues/${taskId}" style="color: var(--gh-link);"><code>${taskId}</code></a>
                    </div>
                  ` : ''}
                  <div style="font-size: 12px; color: var(--gh-text-muted);">
                    Commits: <strong>${commits}</strong>
                  </div>
                  <div style="font-size: 12px; color: var(--gh-text-muted);">
                    Last active: ${lastSeen ? new Date(lastSeen).toLocaleTimeString() : 'N/A'}
                  </div>
                </div>
              </div>
            `;
          }).join('')
        }
      </div>
    </div>
  `;
}
