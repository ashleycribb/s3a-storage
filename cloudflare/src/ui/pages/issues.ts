// cloudflare/src/ui/pages/issues.ts
// Issues List and Issue Detail Views (/p/:id/issues and /p/:id/issues/:taskId)

import type { TaskItem } from "../../types";

export interface IssuesPageOptions {
  projectId: string;
  tasks?: TaskItem[];
  filter?: string;
}

export interface IssueDetailPageOptions {
  projectId: string;
  taskId: string;
  task?: TaskItem | null;
  error?: string;
}

export function renderIssuesPage(options: IssuesPageOptions): string {
  const { projectId, tasks = [] } = options;

  const openCount = tasks.filter(t => t.status !== "merged").length;
  const closedCount = tasks.filter(t => t.status === "merged").length;

  return `
    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1rem; gap: 1rem; flex-wrap: wrap;">
      <div style="display: flex; gap: 0.5rem; align-items: center;">
        <span style="font-weight: 600; color: var(--gh-text-heading); font-size: 15px;">Issues</span>
        <span class="gh-badge gh-badge-muted">${tasks.length} total</span>
      </div>
      <div style="display: flex; gap: 0.5rem; align-items: center;">
        <button id="btn-sync-issues" class="gh-btn gh-btn-sm" onclick="syncGitHubIssues()">
          🔄 Sync GitHub Issues
        </button>
        <button class="gh-btn gh-btn-sm gh-btn-primary" onclick="openNewIssueModal()">
          New issue
        </button>
      </div>
    </div>

    <!-- Issues Card List -->
    <div class="gh-card">
      <div class="gh-card-header" style="background: #161b22;">
        <div style="display: flex; gap: 1rem; align-items: center; font-size: 13px;">
          <a href="/p/${projectId}/issues" style="font-weight: 600; color: var(--gh-text-heading); display: flex; align-items: center; gap: 0.35rem;">
            <svg height="16" viewBox="0 0 16 16" width="16" fill="var(--gh-green-text)"><path d="M8 9.5a1.5 1.5 0 100-3 1.5 1.5 0 000 3z"/><path fill-rule="evenodd" d="M8 0a8 8 0 100 16A8 8 0 008 0zM1.5 8a6.5 6.5 0 1113 0 6.5 6.5 0 01-13 0z"/></svg>
            ${openCount} Open
          </a>
          <span style="color: var(--gh-text-muted); display: flex; align-items: center; gap: 0.35rem;">
            <svg height="16" viewBox="0 0 16 16" width="16" fill="var(--gh-purple-text)"><path fill-rule="evenodd" d="M13.78 4.22a.75.75 0 010 1.06l-7.25 7.25a.75.75 0 01-1.06 0L2.22 9.28a.75.75 0 011.06-1.06L6 10.94l6.72-6.72a.75.75 0 011.06 0z"/></svg>
            ${closedCount} Closed
          </span>
        </div>
      </div>

      <div style="display: flex; flex-direction: column;">
        ${tasks.length === 0
          ? `
            <div class="gh-blankslate">
              <div class="gh-blankslate-icon">📋</div>
              <h3>No open issues</h3>
              <p>There aren't any open issues right now. Create a new engineering task or sync with GitHub.</p>
              <div style="display: flex; gap: 0.5rem; justify-content: center;">
                <button class="gh-btn gh-btn-primary" onclick="openNewIssueModal()">New issue</button>
                <button class="gh-btn" onclick="syncGitHubIssues()">Sync GitHub Issues</button>
              </div>
            </div>
          `
          : tasks.map(task => {
            const isClosed = task.status === "merged";
            const isClaimed = !!task.claimed_by;
            const isHighRisk = task.risk_level === "high";
            const issueNum = task.github_issue_number || task.id;

            return `
              <div class="gh-row" style="align-items: flex-start; gap: 0.75rem;">
                <div style="margin-top: 3px;">
                  ${isClosed ? `
                    <svg width="16" height="16" viewBox="0 0 16 16" fill="var(--gh-purple-text)"><path fill-rule="evenodd" d="M13.78 4.22a.75.75 0 010 1.06l-7.25 7.25a.75.75 0 01-1.06 0L2.22 9.28a.75.75 0 011.06-1.06L6 10.94l6.72-6.72a.75.75 0 011.06 0z"/></svg>
                  ` : `
                    <svg width="16" height="16" viewBox="0 0 16 16" fill="var(--gh-green-text)"><path d="M8 9.5a1.5 1.5 0 100-3 1.5 1.5 0 000 3z"/><path fill-rule="evenodd" d="M8 0a8 8 0 100 16A8 8 0 008 0zM1.5 8a6.5 6.5 0 1113 0 6.5 6.5 0 01-13 0z"/></svg>
                  `}
                </div>
                <div style="flex: 1;">
                  <div style="display: flex; align-items: center; gap: 0.4rem; flex-wrap: wrap;">
                    <a href="/p/${projectId}/issues/${task.id}" style="font-weight: 600; font-size: 14.5px; color: var(--gh-text-heading);">
                      ${task.title}
                    </a>
                    ${isHighRisk ? '<span class="gh-badge gh-badge-danger">needs-human</span>' : ''}
                    ${task.status === "changes_requested" ? '<span class="gh-badge gh-badge-yellow">Changes Requested</span>' : ''}
                    ${task.status === "deferred_budget" ? '<span class="gh-badge gh-badge-yellow">Deferred Budget</span>' : ''}
                    ${isClaimed ? `<span class="gh-badge gh-badge-blue">Claimed: ${task.claimed_by}</span>` : ''}
                  </div>
                  <div style="font-size: 12px; color: var(--gh-text-muted); margin-top: 0.25rem;">
                    #${issueNum} &bull; Task ID: <code>${task.id}</code> &bull; Risk: <code>${task.risk_level}</code> &bull; Status: <code>${task.status}</code>
                  </div>
                </div>
                <div style="display: flex; gap: 0.4rem; align-items: center;">
                  <a href="/p/${projectId}/issues/${task.id}" class="gh-btn gh-btn-sm">
                    View Issue ↗
                  </a>
                </div>
              </div>
            `;
          }).join('')
        }
      </div>
    </div>

    <!-- Modal: Create New Issue -->
    <div id="new-issue-modal" class="modal-overlay">
      <div style="background: var(--gh-card-bg); border: 1px solid var(--gh-border); border-radius: 8px; width: 100%; max-width: 600px; overflow: hidden; box-shadow: 0 20px 40px rgba(0,0,0,0.85);">
        <div style="padding: 1rem 1.25rem; border-bottom: 1px solid var(--gh-border); display: flex; justify-content: space-between; align-items: center; background: #0d1117;">
          <strong style="color: var(--gh-text-heading); font-size: 1.05rem;">Create New Task Issue</strong>
          <button onclick="closeNewIssueModal()" style="background: transparent; border: none; color: var(--gh-text-muted); font-size: 1.2rem; cursor: pointer;">✕</button>
        </div>
        <div style="padding: 1.25rem; font-size: 13px;">
          <div style="margin-bottom: 1rem;">
            <label style="display: block; font-weight: 600; margin-bottom: 0.35rem; color: var(--gh-text-heading);">Issue Title:</label>
            <input type="text" id="new-issue-title" class="gh-input" placeholder="e.g. feat(auth): add spectator role permissions">
          </div>
          <div style="margin-bottom: 1rem;">
            <label style="display: block; font-weight: 600; margin-bottom: 0.35rem; color: var(--gh-text-heading);">Description &amp; Requirements:</label>
            <textarea id="new-issue-desc" class="gh-textarea" rows="4" placeholder="Describe the feature or bug fix requirements..."></textarea>
          </div>
          <div style="margin-bottom: 1rem;">
            <label style="display: block; font-weight: 600; margin-bottom: 0.35rem; color: var(--gh-text-heading);">Allowed File Scope (comma-separated):</label>
            <input type="text" id="new-issue-files" class="gh-input" value="apps/brakets-gg/app/api/auth/, packages/auth/">
          </div>
          <div style="display: flex; gap: 1rem; margin-bottom: 1.25rem;">
            <div style="flex: 1;">
              <label style="display: block; font-weight: 600; margin-bottom: 0.35rem; color: var(--gh-text-heading);">Verification Test Stage:</label>
              <input type="text" id="new-issue-tests" class="gh-input" value="npm run build">
            </div>
            <div style="width: 150px;">
              <label style="display: block; font-weight: 600; margin-bottom: 0.35rem; color: var(--gh-text-heading);">Risk Level:</label>
              <select id="new-issue-risk" class="gh-select">
                <option value="low">Low Risk</option>
                <option value="high">High (Human Gated)</option>
              </select>
            </div>
          </div>
          <div style="display: flex; justify-content: flex-end; gap: 0.5rem;">
            <button class="gh-btn" onclick="closeNewIssueModal()">Cancel</button>
            <button id="btn-submit-issue" class="gh-btn gh-btn-primary" onclick="submitNewIssue()">Submit Issue</button>
          </div>
        </div>
      </div>
    </div>

    <script>
      function openNewIssueModal() {
        const modal = document.getElementById('new-issue-modal');
        if (modal) modal.style.display = 'flex';
      }
      function closeNewIssueModal() {
        const modal = document.getElementById('new-issue-modal');
        if (modal) modal.style.display = 'none';
      }

      async function submitNewIssue() {
        const titleInput = document.getElementById('new-issue-title');
        const descInput = document.getElementById('new-issue-desc');
        const filesInput = document.getElementById('new-issue-files');
        const testsInput = document.getElementById('new-issue-tests');
        const riskInput = document.getElementById('new-issue-risk');
        const btn = document.getElementById('btn-submit-issue');

        const title = titleInput.value.trim();
        if (!title) {
          alert('Please enter an issue title.');
          return;
        }

        btn.disabled = true;
        btn.textContent = '⏳ Creating issue...';

        try {
          const res = await fetch('/project/${projectId}/tasks/create', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
              title,
              description: descInput.value.trim(),
              files_allowed: filesInput.value.split(',').map(s => s.trim()).filter(Boolean),
              tests: testsInput.value.trim() || 'npm run build',
              risk_level: riskInput.value,
            })
          });

          if (res.ok) {
            const data = await res.json();
            window.location.href = '/p/${projectId}/issues/' + (data.task?.id || '');
          } else {
            alert('Failed to create issue: ' + await res.text());
            btn.disabled = false;
            btn.textContent = 'Submit Issue';
          }
        } catch (err) {
          alert('Error creating issue: ' + err.message);
          btn.disabled = false;
          btn.textContent = 'Submit Issue';
        }
      }

      async function syncGitHubIssues() {
        const btn = document.getElementById('btn-sync-issues');
        if (btn) { btn.disabled = true; btn.textContent = '⏳ Syncing...'; }
        try {
          const res = await fetch('/project/${projectId}/tasks/sync', { method: 'POST' });
          if (res.ok) {
            const data = await res.json();
            alert('GitHub issues synced successfully.');
            location.reload();
          } else {
            alert('Failed to sync issues: ' + await res.text());
            if (btn) { btn.disabled = false; btn.textContent = '🔄 Sync GitHub Issues'; }
          }
        } catch (e) {
          alert('Sync error: ' + e.message);
          if (btn) { btn.disabled = false; btn.textContent = '🔄 Sync GitHub Issues'; }
        }
      }
    </script>
  `;
}

export function renderIssueDetailPage(options: IssueDetailPageOptions): string {
  const { projectId, taskId, task, error } = options;

  if (error || !task) {
    return `
      <div class="gh-blankslate">
        <div class="gh-blankslate-icon">⚠️</div>
        <h3>Issue not found</h3>
        <p>${error || `Could not find issue #${taskId} in project ${projectId}.`}</p>
        <a href="/p/${projectId}/issues" class="gh-btn gh-btn-primary">
          ← Back to Issues
        </a>
      </div>
    `;
  }

  const isClosed = task.status === "merged";
  const isClaimed = !!task.claimed_by;
  const isHighRisk = task.risk_level === "high";

  return `
    <div style="margin-bottom: 1.5rem;">
      <div style="display: flex; justify-content: space-between; align-items: flex-start; gap: 1rem; flex-wrap: wrap;">
        <div>
          <h1 style="font-size: 1.5rem; font-weight: 600; color: var(--gh-text-heading); margin-bottom: 0.5rem;">
            ${task.title} <span style="color: var(--gh-text-muted); font-weight: 400;">#${task.github_issue_number || task.id}</span>
          </h1>
          <div style="display: flex; gap: 0.6rem; align-items: center; font-size: 13px; color: var(--gh-text-muted); flex-wrap: wrap;">
            ${isClosed ? `
              <span class="gh-badge gh-badge-purple">✓ Closed</span>
            ` : isClaimed ? `
              <span class="gh-badge gh-badge-blue">In Progress</span>
            ` : `
              <span class="gh-badge gh-badge-success">Open</span>
            `}
            <span>Opened by <strong>PM</strong> &bull; Status: <code>${task.status}</code></span>
            ${isHighRisk ? '<span class="gh-badge gh-badge-danger">needs-human</span>' : ''}
          </div>
        </div>

        <div style="display: flex; gap: 0.5rem; align-items: center;">
          <a href="/p/${projectId}/issues" class="gh-btn gh-btn-sm">
            ← All Issues
          </a>
          <button id="btn-reset-task" class="gh-btn gh-btn-sm" onclick="resetTask('${task.id}')">
            🔄 Reset Task to Open
          </button>
        </div>
      </div>
    </div>

    <!-- Main Content & Sidebar -->
    <div style="display: flex; gap: 1.5rem; flex-wrap: wrap;">
      <div style="flex: 1; min-width: 320px;">
        <div class="gh-card">
          <div class="gh-card-header">
            <span style="font-weight: 600; color: var(--gh-text-heading); font-size: 13px;">Task Description</span>
          </div>
          <div class="gh-card-body" style="font-size: 14px; line-height: 1.6; color: var(--gh-text-heading); white-space: pre-wrap;">${task.description}</div>
        </div>

        ${task.feedback ? `
          <div class="gh-card" style="border-color: rgba(210,153,34,0.4);">
            <div class="gh-card-header" style="background: rgba(210,153,34,0.1);">
              <span style="font-weight: 600; color: var(--gh-yellow-text); font-size: 13px;">Supervisor Feedback</span>
            </div>
            <div class="gh-card-body" style="font-style: italic; color: #fde68a;">
              "${task.feedback}"
            </div>
          </div>
        ` : ''}

        ${task.change_request_id ? `
          <div class="gh-card">
            <div class="gh-card-header">
              <span style="font-weight: 600; color: var(--gh-text-heading); font-size: 13px;">Associated Change Request</span>
            </div>
            <div class="gh-card-body" style="display: flex; justify-content: space-between; align-items: center;">
              <div>
                <span style="font-weight: 600; color: var(--gh-text-heading);">Change Request #${task.change_request_id}</span>
                <p style="color: var(--gh-text-muted); font-size: 12px; margin-top: 0.2rem;">Generated by autonomous agent for this issue.</p>
              </div>
              <a href="/p/${projectId}/pulls/${task.change_request_id}" class="gh-btn gh-btn-primary">
                ⚖️ Review Change Request ↗
              </a>
            </div>
          </div>
        ` : ''}
      </div>

      <!-- Sidebar -->
      <div style="width: 300px; flex-shrink: 0;">
        <div class="gh-card">
          <div class="gh-card-header">
            <strong style="color: var(--gh-text-heading); font-size: 13px;">Task Metadata</strong>
          </div>
          <div class="gh-card-body" style="font-size: 13px;">
            <div style="margin-bottom: 0.8rem;">
              <span style="color: var(--gh-text-muted); display: block; font-size: 11px; text-transform: uppercase; font-weight: 600;">Assignee</span>
              <div style="margin-top: 0.2rem;">
                ${task.claimed_by ? `<span class="gh-badge gh-badge-blue">🤖 Agent ${task.claimed_by}</span>` : '<span style="color: var(--gh-text-muted);">Unassigned (Waiting for Swarm)</span>'}
              </div>
            </div>

            <div style="margin-bottom: 0.8rem;">
              <span style="color: var(--gh-text-muted); display: block; font-size: 11px; text-transform: uppercase; font-weight: 600;">Allowed File Scope</span>
              <div style="margin-top: 0.2rem; font-family: monospace; font-size: 12px; color: var(--gh-link);">
                ${task.files_allowed && task.files_allowed.length > 0 ? task.files_allowed.join('<br>') : 'All repository files'}
              </div>
            </div>

            <div>
              <span style="color: var(--gh-text-muted); display: block; font-size: 11px; text-transform: uppercase; font-weight: 600;">Test Command</span>
              <code style="font-size: 12px; color: #38bdf8;">${task.tests || 'npm run build'}</code>
            </div>
          </div>
        </div>
      </div>
    </div>

    <script>
      async function resetTask(taskId) {
        if (!confirm('Reset task ' + taskId + ' to OPEN and unclaim any assigned agent?')) return;
        const btn = document.getElementById('btn-reset-task');
        if (btn) { btn.disabled = true; btn.textContent = '⏳ Resetting...'; }
        try {
          const res = await fetch('/project/${projectId}/reset-task/' + taskId, { method: 'POST' });
          if (res.ok) {
            alert('Task ' + taskId + ' reset to OPEN.');
            location.reload();
          } else {
            alert('Failed to reset task: ' + await res.text());
            if (btn) { btn.disabled = false; btn.textContent = '🔄 Reset Task to Open'; }
          }
        } catch (err) {
          alert('Error: ' + err.message);
          if (btn) { btn.disabled = false; btn.textContent = '🔄 Reset Task to Open'; }
        }
      }
    </script>
  `;
}
