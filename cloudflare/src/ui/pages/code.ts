// cloudflare/src/ui/pages/code.ts
// Repository Code & Artifacts View

export interface CodePageOptions {
  projectId: string;
  statusData?: any;
}

export function renderCodePage(options: CodePageOptions): string {
  const { projectId, statusData = {} } = options;
  const commitsCount = statusData.commits_count || 0;
  const autoMerged = statusData.auto_merged || 0;
  const baselineRepo = statusData.baseline_repo || { name: `s3a-baseline-${projectId}`, remote: "" };

  return `
    <div style="display: flex; gap: 1.5rem; flex-wrap: wrap;">
      <!-- Main Repository Content -->
      <div style="flex: 1; min-width: 320px;">
        <!-- Repo Stats Banner -->
        <div class="gh-card" style="margin-bottom: 1rem;">
          <div class="gh-card-header" style="background: #161b22;">
            <div style="display: flex; align-items: center; gap: 0.6rem;">
              <span class="gh-badge gh-badge-blue">Repository Baseline</span>
              <strong style="color: var(--gh-text-heading); font-size: 13px;">${baselineRepo.name}</strong>
            </div>
            <div style="font-size: 12px; color: var(--gh-text-muted);">
              Branch: <code style="color: #38bdf8;">main</code> &bull; Single-Writer Push Ordering
            </div>
          </div>
          <div class="gh-card-body" style="padding: 0.85rem 1rem;">
            <div style="display: flex; gap: 1.5rem; flex-wrap: wrap;">
              <div>
                <span style="color: var(--gh-text-muted); font-size: 12px;">Total Commits</span>
                <div style="font-size: 1.25rem; font-weight: 600; color: var(--gh-text-heading);">${commitsCount}</div>
              </div>
              <div>
                <span style="color: var(--gh-text-muted); font-size: 12px;">Auto-Merged</span>
                <div style="font-size: 1.25rem; font-weight: 600; color: var(--gh-green-text);">${autoMerged}</div>
              </div>
              <div>
                <span style="color: var(--gh-text-muted); font-size: 12px;">Live Preview</span>
                <div style="margin-top: 0.2rem;">
                  <a href="/project/${projectId}/preview/live" target="_blank" class="gh-btn gh-btn-sm" style="color: var(--gh-orange); border-color: rgba(243,128,32,0.4);">
                    🌐 Open live preview ↗
                  </a>
                </div>
              </div>
            </div>
          </div>
        </div>

        <!-- Baseline Files Table -->
        <div class="gh-card">
          <div class="gh-card-header">
            <span style="font-weight: 600; color: var(--gh-text-heading); font-size: 13px;">Repository Files &amp; Artifacts</span>
            <span style="font-size: 12px; color: var(--gh-text-muted);">Cloudflare Artifacts Namespace: <code>default</code></span>
          </div>

          <div style="display: flex; flex-direction: column;">
            <div class="gh-row" style="background: #111827; font-size: 12px; font-weight: 600; color: var(--gh-text-muted);">
              <span style="flex: 2;">File Path</span>
              <span style="flex: 3;">Latest Verification Stage</span>
              <span style="flex: 1; text-align: right;">Status</span>
            </div>

            <div class="gh-row">
              <span style="flex: 2; font-family: monospace; font-size: 13px; color: var(--gh-link);">apps/brakets-gg/app/api/auth/roles.ts</span>
              <span style="flex: 3; font-size: 12px; color: var(--gh-text-muted);">Role-aware authentication &amp; tournament permissions</span>
              <span style="flex: 1; text-align: right;"><span class="gh-badge gh-badge-success">Verified</span></span>
            </div>

            <div class="gh-row">
              <span style="flex: 2; font-family: monospace; font-size: 13px; color: var(--gh-link);">packages/auth/permissions.ts</span>
              <span style="flex: 3; font-size: 12px; color: var(--gh-text-muted);">PlatformRole permission matrix and guardrails</span>
              <span style="flex: 1; text-align: right;"><span class="gh-badge gh-badge-success">Verified</span></span>
            </div>

            <div class="gh-row">
              <span style="flex: 2; font-family: monospace; font-size: 13px; color: var(--gh-link);">apps/brakets-gg/package.json</span>
              <span style="flex: 3; font-size: 12px; color: var(--gh-text-muted);">Brakets tournament dashboard dependencies</span>
              <span style="flex: 1; text-align: right;"><span class="gh-badge gh-badge-muted">Base</span></span>
            </div>

            <div class="gh-row">
              <span style="flex: 2; font-family: monospace; font-size: 13px; color: var(--gh-link);">lib/escrow.ts</span>
              <span style="flex: 3; font-size: 12px; color: var(--gh-text-muted);">Prize pool escrow &amp; payout guardrails</span>
              <span style="flex: 1; text-align: right;"><span class="gh-badge gh-badge-yellow">Needs-Human</span></span>
            </div>
          </div>
        </div>

        <!-- Recent Activity Log -->
        <div class="gh-card">
          <div class="gh-card-header">
            <span style="font-weight: 600; color: var(--gh-text-heading); font-size: 13px;">Recent Commits &amp; Activity</span>
            <span style="font-size: 12px; color: var(--gh-text-muted);">Append-only S3A git graph</span>
          </div>
          <div style="display: flex; flex-direction: column;">
            ${(statusData.recent_merges && statusData.recent_merges.length > 0)
              ? statusData.recent_merges.map((m: any) => `
                <div class="gh-row">
                  <div>
                    <div style="font-weight: 500; color: var(--gh-text-heading); font-size: 13.5px;">${m.message || 'Commit'}</div>
                    <div style="font-size: 12px; color: var(--gh-text-muted); margin-top: 0.2rem;">
                      Commit <code>${(m.commit_id || '').slice(0, 10)}</code> &bull; Author: <code>${m.author_gate || 'agent'}</code>
                    </div>
                  </div>
                  <span class="gh-badge gh-badge-purple">Merged</span>
                </div>
              `).join('')
              : `
                <div class="gh-blankslate" style="padding: 2rem;">
                  <div class="gh-blankslate-icon">📦</div>
                  <h3>No custom agent commits yet</h3>
                  <p>Baseline repository is loaded and ready for autonomous agent task execution.</p>
                </div>
              `
            }
          </div>
        </div>
      </div>

      <!-- Right Sidebar: Quick Info & Rules -->
      <div style="width: 320px; flex-shrink: 0;">
        <div class="gh-card">
          <div class="gh-card-header">
            <strong style="color: var(--gh-text-heading); font-size: 13px;">About</strong>
          </div>
          <div class="gh-card-body" style="font-size: 13px; color: var(--gh-text-muted); line-height: 1.5;">
            <p style="margin-bottom: 0.8rem;">
              <strong>S3A Gate</strong>: Autonomous agent VCS and verification gate for Brakets (ashleycribb/plat-gaming).
            </p>
            <p style="margin-bottom: 0.8rem;">
              Autonomous agents detect, measure and propose changes. Humans make all approval and deployment judgments.
            </p>
            <div style="border-top: 1px solid var(--gh-border); padding-top: 0.8rem; margin-top: 0.8rem;">
              <div style="font-weight: 600; color: var(--gh-text-heading); margin-bottom: 0.4rem;">Gating Invariants:</div>
              <ul style="padding-left: 1.2rem; font-size: 12px;">
                <li>Files in <code>cashier/escrow/auth/db</code> require 2 humans</li>
                <li>Zero hard-coded fake test passes</li>
                <li>Single-Writer push ordering per project</li>
              </ul>
            </div>
          </div>
        </div>
      </div>
    </div>
  `;
}
