// cloudflare/src/ui/pages/pulls.ts
// Pull Requests List and PR Detail / Judgment Views (/p/:id/pulls and /p/:id/pulls/:crId)

import type { ChangeRequest } from "../../types";

export interface PullsPageOptions {
  projectId: string;
  changeRequests?: ChangeRequest[];
  filter?: string;
}

export interface PullDetailPageOptions {
  projectId: string;
  crId: string;
  cr?: ChangeRequest | null;
  error?: string;
}

export function renderPullsPage(options: PullsPageOptions): string {
  const { projectId, changeRequests = [] } = options;

  const openCount = changeRequests.filter(c => c.status === "pending_review" || c.status === "changes_requested").length;
  const mergedCount = changeRequests.filter(c => c.status === "auto_merged" || c.status === "approved").length;
  const rejectedCount = changeRequests.filter(c => c.status === "rejected").length;

  return `
    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1rem; gap: 1rem; flex-wrap: wrap;">
      <div style="display: flex; gap: 0.5rem; align-items: center;">
        <span style="font-weight: 600; color: var(--gh-text-heading); font-size: 15px;">Pull requests</span>
        <span class="gh-badge gh-badge-muted">${changeRequests.length} total</span>
      </div>
      <div style="display: flex; gap: 0.5rem; align-items: center;">
        <button id="btn-triage-dups" class="gh-btn gh-btn-sm" onclick="closeDuplicatePRs()">
          🧹 Triage Dups
        </button>
        <a href="/project/${projectId}/preview/live" target="_blank" class="gh-btn gh-btn-sm" style="color: var(--gh-orange); border-color: rgba(243,128,32,0.4);">
          🌐 Open live preview ↗
        </a>
      </div>
    </div>

    <!-- Pull Requests Card List -->
    <div class="gh-card">
      <div class="gh-card-header" style="background: #161b22;">
        <div style="display: flex; gap: 1rem; align-items: center; font-size: 13px;">
          <a href="/p/${projectId}/pulls" style="font-weight: 600; color: var(--gh-text-heading); display: flex; align-items: center; gap: 0.35rem;">
            <svg height="16" viewBox="0 0 16 16" width="16" fill="var(--gh-green-text)"><path fill-rule="evenodd" d="M7.177 3.073L9.573.677A.25.25 0 0110 .854v4.792a.25.25 0 01-.427.177L7.177 3.427a.25.25 0 010-.354zM3.75 2.5a.75.75 0 100 1.5.75.75 0 000-1.5zm-2.25.75a2.25 2.25 0 113 2.122v5.256a2.251 2.251 0 11-1.5 0V5.372A2.25 2.25 0 011.5 3.25zM11 2.5h-1V4h1a1 1 0 011 1v5.628a2.251 2.251 0 101.5 0V5A2.5 2.5 0 0011 2.5zm1 10.25a.75.75 0 111.5 0 .75.75 0 01-1.5 0zM3.75 12a.75.75 0 100 1.5.75.75 0 000-1.5z"/></svg>
            ${openCount} Open
          </a>
          <span style="color: var(--gh-text-muted); display: flex; align-items: center; gap: 0.35rem;">
            <svg height="16" viewBox="0 0 16 16" width="16" fill="var(--gh-purple-text)"><path fill-rule="evenodd" d="M13.78 4.22a.75.75 0 010 1.06l-7.25 7.25a.75.75 0 01-1.06 0L2.22 9.28a.75.75 0 011.06-1.06L6 10.94l6.72-6.72a.75.75 0 011.06 0z"/></svg>
            ${mergedCount} Merged
          </span>
          <span style="color: var(--gh-text-muted); display: flex; align-items: center; gap: 0.35rem;">
            <svg height="16" viewBox="0 0 16 16" width="16" fill="var(--gh-red-text)"><path fill-rule="evenodd" d="M3.72 3.72a.75.75 0 011.06 0L8 6.94l3.22-3.22a.75.75 0 111.06 1.06L9.06 8l3.22 3.22a.75.75 0 11-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 01-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 010-1.06z"/></svg>
            ${rejectedCount} Rejected
          </span>
        </div>
      </div>

      <div style="display: flex; flex-direction: column;">
        ${changeRequests.length === 0
          ? `
            <div class="gh-blankslate">
              <div class="gh-blankslate-icon">🔀</div>
              <h3>No pull requests found</h3>
              <p>Autonomous agents have not opened any change requests yet. When agents complete work, their change requests appear here for human supervisor review.</p>
            </div>
          `
          : changeRequests.map(cr => {
            const isApproved = cr.status === "approved" || cr.status === "auto_merged";
            const isRejected = cr.status === "rejected";
            const isHighRisk = cr.risk_level === "high" || cr.requires_human;
            const hasRealQA = cr.qa_reports && cr.qa_reports.length > 0;

            return `
              <div class="gh-row" style="flex-direction: column; gap: 0.65rem; align-items: stretch;">
                <div style="display: flex; justify-content: space-between; align-items: flex-start; width: 100%; gap: 1rem; flex-wrap: wrap;">
                  <div style="display: flex; align-items: flex-start; gap: 0.65rem;">
                    <div style="margin-top: 2px;">
                      ${isApproved ? `
                        <svg width="16" height="16" viewBox="0 0 16 16" fill="var(--gh-purple-text)"><path fill-rule="evenodd" d="M13.78 4.22a.75.75 0 010 1.06l-7.25 7.25a.75.75 0 01-1.06 0L2.22 9.28a.75.75 0 011.06-1.06L6 10.94l6.72-6.72a.75.75 0 011.06 0z"/></svg>
                      ` : isRejected ? `
                        <svg width="16" height="16" viewBox="0 0 16 16" fill="var(--gh-red-text)"><path fill-rule="evenodd" d="M3.72 3.72a.75.75 0 011.06 0L8 6.94l3.22-3.22a.75.75 0 111.06 1.06L9.06 8l3.22 3.22a.75.75 0 11-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 01-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 010-1.06z"/></svg>
                      ` : `
                        <svg width="16" height="16" viewBox="0 0 16 16" fill="var(--gh-green-text)"><path fill-rule="evenodd" d="M7.177 3.073L9.573.677A.25.25 0 0110 .854v4.792a.25.25 0 01-.427.177L7.177 3.427a.25.25 0 010-.354zM3.75 2.5a.75.75 0 100 1.5.75.75 0 000-1.5zm-2.25.75a2.25 2.25 0 113 2.122v5.256a2.251 2.251 0 11-1.5 0V5.372A2.25 2.25 0 011.5 3.25zM11 2.5h-1V4h1a1 1 0 011 1v5.628a2.251 2.251 0 101.5 0V5A2.5 2.5 0 0011 2.5zm1 10.25a.75.75 0 111.5 0 .75.75 0 01-1.5 0zM3.75 12a.75.75 0 100 1.5.75.75 0 000-1.5z"/></svg>
                      `}
                    </div>
                    <div>
                      <div style="display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap;">
                        <a href="/p/${projectId}/pulls/${cr.id}" style="font-weight: 600; font-size: 14.5px; color: var(--gh-text-heading);">
                          ${cr.title}
                        </a>
                        ${isApproved ? '<span class="gh-badge gh-badge-purple">Merged</span>' : isRejected ? '<span class="gh-badge gh-badge-danger">Rejected</span>' : '<span class="gh-badge gh-badge-success">Open (Review)</span>'}
                        ${isHighRisk ? '<span class="gh-badge gh-badge-danger">needs-human</span>' : ''}
                      </div>
                      <div style="font-size: 12px; color: var(--gh-text-muted); margin-top: 0.25rem;">
                        #${cr.id} opened by agent <code>${cr.agent_id}</code> &bull; Task: <code>${cr.task_id}</code> &bull; Commit: <code>${(cr.commit_id || '').slice(0, 10)}</code> &bull; Touched: <code>${(cr.touched_files || []).join(', ')}</code>
                      </div>
                    </div>
                  </div>

                  <!-- Actions / Links -->
                  <div style="display: flex; gap: 0.4rem; align-items: center; flex-wrap: wrap;">
                    <a href="/project/${projectId}/preview/live" target="_blank" class="gh-btn gh-btn-sm" style="color: var(--gh-orange); border-color: rgba(243,128,32,0.4);">
                      🌐 Open preview ↗
                    </a>
                    <a href="/p/${projectId}/pulls/${cr.id}" class="gh-btn gh-btn-sm gh-btn-primary">
                      ⚖️ Review &amp; Judge ↗
                    </a>
                    ${cr.github_pr_url ? `
                      <a href="${cr.github_pr_url}" target="_blank" class="gh-btn gh-btn-sm">
                        GitHub PR #${cr.github_pr_number || ''} ↗
                      </a>
                    ` : ''}
                  </div>
                </div>

                <!-- Checks Summary -->
                <div style="display: flex; gap: 0.65rem; align-items: center; padding-left: 1.65rem; font-size: 12px; flex-wrap: wrap;">
                  <span style="color: var(--gh-text-muted);">Checks:</span>
                  <span class="gh-badge ${cr.test_report?.passed ? 'gh-badge-success' : 'gh-badge-danger'}">
                    ${cr.test_report?.passed ? '✓ Sandbox build passed' : '✗ Build failed'}
                  </span>
                  <span class="gh-badge ${hasRealQA ? 'gh-badge-success' : 'gh-badge-muted'}">
                    ${hasRealQA ? `✓ Browser QA passed (${cr.qa_reports?.length || 0} run)` : 'QA not run'}
                  </span>
                  ${cr.human_rationale ? `
                    <span style="color: var(--gh-yellow-text); font-style: italic;">
                      Supervisor: "${cr.human_rationale}"
                    </span>
                  ` : ''}
                </div>
              </div>
            `;
          }).join('')
        }
      </div>
    </div>

    <script>
      async function closeDuplicatePRs() {
        if (!confirm('Close all duplicate s3a/agent-* PRs for #3 except oldest canonical PR #82?')) return;
        const btn = document.getElementById('btn-triage-dups');
        if (btn) { btn.disabled = true; btn.textContent = '⏳ Triaging...'; }
        try {
          const res = await fetch('/project/${projectId}/close-duplicate-prs?issue=3&preserve=82', {
            method: 'POST',
            headers: getAdminAuthHeaders(),
          });
          if (res.status === 401) {
            if (btn) { btn.disabled = false; btn.textContent = '🧹 Triage Dups'; }
            const sec = await promptAndStoreAdminSecret();
            if (sec) closeDuplicatePRs();
            return;
          }
          const data = await res.json();
          alert('PR Triage Result:\\n- Preserved: #' + data.preserved + '\\n- Closed: ' + (data.closed?.length || 0));
          location.reload();
        } catch (e) {
          alert('Error closing duplicate PRs: ' + e.message);
          if (btn) { btn.disabled = false; btn.textContent = '🧹 Triage Dups'; }
        }
      }
    </script>
  `;
}

export function renderPullDetailPage(options: PullDetailPageOptions): string {
  const { projectId, crId, cr, error } = options;

  if (error || !cr) {
    return `
      <div class="gh-blankslate">
        <div class="gh-blankslate-icon">⚠️</div>
        <h3>Pull request not found</h3>
        <p>${error || `Could not find change request #${crId} in project ${projectId}.`}</p>
        <a href="/p/${projectId}/pulls" class="gh-btn gh-btn-primary">
          ← Back to Pull Requests
        </a>
      </div>
    `;
  }

  const isApproved = cr.status === "approved" || cr.status === "auto_merged";
  const isRejected = cr.status === "rejected";
  const isHighRisk = cr.risk_level === "high" || cr.requires_human;
  const hasLogs = !!(cr.build_log || (cr.test_report && cr.test_report.output));
  const hasQa = !!(cr.qa_reports && cr.qa_reports.length > 0);

  return `
    <div style="margin-bottom: 1.5rem;">
      <div style="display: flex; justify-content: space-between; align-items: flex-start; gap: 1rem; flex-wrap: wrap;">
        <div>
          <h1 style="font-size: 1.5rem; font-weight: 600; color: var(--gh-text-heading); margin-bottom: 0.5rem;">
            ${cr.title} <span style="color: var(--gh-text-muted); font-weight: 400;">#${cr.github_pr_number || cr.id}</span>
          </h1>
          <div style="display: flex; gap: 0.6rem; align-items: center; font-size: 13px; color: var(--gh-text-muted); flex-wrap: wrap;">
            ${isApproved ? `
              <span class="gh-badge gh-badge-purple">✓ Merged</span>
            ` : isRejected ? `
              <span class="gh-badge gh-badge-danger">✗ Rejected</span>
            ` : `
              <span class="gh-badge gh-badge-success">Open (Ready for Judgment)</span>
            `}
            <span>Opened by Agent <code>${cr.agent_id}</code> &bull; Task: <a href="/p/${projectId}/issues/${cr.task_id}"><code>${cr.task_id}</code></a> &bull; Commit: <code>${(cr.commit_id || '').slice(0, 10)}</code></span>
            ${isHighRisk ? '<span class="gh-badge gh-badge-danger">needs-human</span>' : ''}
          </div>
        </div>

        <div style="display: flex; gap: 0.5rem; align-items: center;">
          <a href="/p/${projectId}/pulls" class="gh-btn gh-btn-sm">
            ← All Pull Requests
          </a>
          <a href="/project/${projectId}/preview/live" target="_blank" class="gh-btn gh-btn-sm" style="color: var(--gh-orange); border-color: rgba(243,128,32,0.4);">
            🌐 Open live preview ↗
          </a>
        </div>
      </div>
    </div>

    <!-- Main Content & Judgment Panel -->
    <div style="display: flex; gap: 1.5rem; flex-wrap: wrap;">
      <!-- Left: Diff, Build Verification, and QA Evidence -->
      <div style="flex: 1; min-width: 320px;">
        <!-- Agent Commit Message -->
        <div class="gh-card">
          <div class="gh-card-header">
            <span style="font-weight: 600; color: var(--gh-text-heading); font-size: 13px;">Agent Summary &amp; Commit Rationale</span>
          </div>
          <div class="gh-card-body" style="font-size: 13.5px; line-height: 1.6; color: var(--gh-text-heading); white-space: pre-wrap;">${cr.message || 'No description provided.'}</div>
        </div>

        <!-- Build & Test Verification Card -->
        <div class="gh-card">
          <div class="gh-card-header">
            <span style="font-weight: 600; color: var(--gh-text-heading); font-size: 13px;">🧪 Build &amp; Test Verification</span>
            <span class="gh-badge ${cr.test_report?.passed ? 'gh-badge-success' : 'gh-badge-danger'}">
              ${cr.test_report?.passed ? '✓ Passed' : '✗ Failed'}
            </span>
          </div>
          <div class="gh-card-body" style="font-size: 13px;">
            ${hasLogs ? `
              <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 0.75rem; margin-bottom: 0.85rem;">
                <div>
                  <span style="color: var(--gh-text-muted); font-size: 11px; text-transform: uppercase;">Command</span>
                  <div style="font-family: monospace; font-size: 12px; color: var(--gh-text-heading);">${cr.test_report?.stage || 'npm run build'}</div>
                </div>
                <div>
                  <span style="color: var(--gh-text-muted); font-size: 11px; text-transform: uppercase;">Exit Code</span>
                  <div style="font-family: monospace; font-size: 12px; color: var(--gh-text-heading);">${cr.test_report?.exit_code !== undefined ? cr.test_report.exit_code : (cr.test_report?.passed ? 0 : 1)}</div>
                </div>
                <div>
                  <span style="color: var(--gh-text-muted); font-size: 11px; text-transform: uppercase;">Duration</span>
                  <div style="font-family: monospace; font-size: 12px; color: var(--gh-text-heading);">${cr.test_report?.duration_ms ? `${cr.test_report.duration_ms}ms` : (cr.test_duration_ms ? `${cr.test_duration_ms}ms` : 'N/A')}</div>
                </div>
              </div>
              ${(cr.test_report?.log_url || cr.build_log_r2_url) ? `
                <div style="margin-bottom: 0.75rem;">
                  <a href="${cr.test_report?.log_url || cr.build_log_r2_url}" target="_blank" class="gh-btn gh-btn-sm">
                    📄 View Full Build &amp; Test Log (R2) ↗
                  </a>
                </div>
              ` : ''}
              <pre style="background: #0d1117; padding: 0.85rem; border-radius: 6px; border: 1px solid var(--gh-border); font-family: monospace; font-size: 12px; overflow-x: auto; color: #7ee787;">${cr.test_report?.output || cr.build_log || 'Build passed'}</pre>
            ` : `
              <div style="color: var(--gh-text-muted); font-style: italic;">
                *Build not run*
              </div>
            `}
          </div>
        </div>

        <!-- Real QA Playwright Evidence Card -->
        <div class="gh-card">
          <div class="gh-card-header">
            <span style="font-weight: 600; color: var(--gh-text-heading); font-size: 13px;">🎭 Real QA Playwright Evidence (Headless Chromium)</span>
            <span class="gh-badge ${hasQa ? 'gh-badge-success' : 'gh-badge-muted'}">
              ${hasQa ? '✓ Verified' : 'QA not run'}
            </span>
          </div>
          <div class="gh-card-body" style="font-size: 13px;">
            ${hasQa ? `
              ${cr.qa_reports!.map(r => `
                <div style="margin-bottom: 1rem; border-bottom: 1px solid var(--gh-border-subtle); padding-bottom: 1rem;">
                  <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.5rem;">
                    <strong>QA Agent: <code>${r.agent_id}</code> (${r.agent_role.toUpperCase()})</strong>
                    <span class="gh-badge ${r.status === 'passed' ? 'gh-badge-success' : 'gh-badge-danger'}">${r.status.toUpperCase()}</span>
                  </div>
                  <ul style="padding-left: 1.2rem; color: var(--gh-text-muted); font-size: 12px; margin-bottom: 0.5rem;">
                    ${(r.passed_steps || []).map(s => `<li>✓ ${s}</li>`).join('')}
                  </ul>
                  ${(r.screenshots || []).length > 0 ? `
                    <div style="display: flex; gap: 0.75rem; flex-wrap: wrap; margin-top: 0.75rem;">
                      ${r.screenshots.map(s => `
                        <div>
                          <a href="${s.data_url}" target="_blank">
                            <img src="${s.data_url}" alt="${s.caption}" style="max-width: 240px; border-radius: 6px; border: 1px solid var(--gh-border);">
                          </a>
                          <div style="font-size: 11px; color: var(--gh-text-muted); margin-top: 0.2rem;">${s.caption}</div>
                        </div>
                      `).join('')}
                    </div>
                  ` : ''}
                </div>
              `).join('')}
            ` : `
              <div style="color: var(--gh-text-muted); font-style: italic;">
                *QA not run*
              </div>
            `}
          </div>
        </div>

        <!-- Touched Files & Code Diff -->
        <div class="gh-card">
          <div class="gh-card-header">
            <span style="font-weight: 600; color: var(--gh-text-heading); font-size: 13px;">Changes Diff</span>
            <span style="font-size: 12px; color: var(--gh-text-muted);">${(cr.touched_files || []).join(', ')}</span>
          </div>
          <div class="gh-card-body" style="padding: 0;">
            <pre style="background: #0d1117; padding: 1rem; margin: 0; font-family: monospace; font-size: 12px; overflow-x: auto; color: var(--gh-text); line-height: 1.5;">${cr.diff || '(No file changes diff attached)'}</pre>
          </div>
        </div>
      </div>

      <!-- Right: Judgment Panel (Human Supervisor Only) -->
      <div style="width: 320px; flex-shrink: 0;">
        <div class="gh-card" style="border-color: rgba(243,128,32,0.5);">
          <div class="gh-card-header" style="background: rgba(243,128,32,0.1);">
            <strong style="color: var(--gh-orange); font-size: 13px;">⚖️ Human Judgment Panel</strong>
          </div>
          <div class="gh-card-body">
            <p style="font-size: 12px; color: var(--gh-text-muted); margin-bottom: 0.85rem; line-height: 1.5;">
              Humans make all final judgments. Approvals are cryptographically signed using your browser's Ed25519 WebCrypto key and verified against <code>SUPERVISOR_PUBKEYS</code>.
            </p>

            ${isApproved ? `
              <div style="background: var(--gh-purple-bg); border: 1px solid rgba(163,113,247,0.4); border-radius: 6px; padding: 0.85rem; text-align: center;">
                <div style="color: var(--gh-purple-text); font-weight: 600; font-size: 13px; margin-bottom: 0.35rem;">✓ Merged into Baseline</div>
                <div style="font-size: 12px; color: var(--gh-text-muted); font-style: italic;">"${cr.human_rationale || 'Approved by human supervisor'}"</div>
              </div>
            ` : isRejected ? `
              <div style="background: var(--gh-red-bg); border: 1px solid rgba(248,81,73,0.4); border-radius: 6px; padding: 0.85rem; text-align: center;">
                <div style="color: var(--gh-red-text); font-weight: 600; font-size: 13px; margin-bottom: 0.35rem;">✗ Rejected</div>
                <div style="font-size: 12px; color: var(--gh-text-muted); font-style: italic;">"${cr.human_rationale || 'Rejected by human supervisor'}"</div>
              </div>
            ` : `
              <div style="margin-bottom: 0.85rem;">
                <label style="display: block; font-weight: 600; font-size: 12px; margin-bottom: 0.35rem; color: var(--gh-text-heading);">Supervisor Rationale / Feedback:</label>
                <textarea id="pm-feedback-input" class="gh-textarea" rows="3" placeholder="Enter reason for approval, or feedback for revisions..."></textarea>
              </div>

              <div style="display: flex; flex-direction: column; gap: 0.5rem;">
                <button id="btn-approve-cr" class="gh-btn gh-btn-primary" style="justify-content: center;" onclick="approveCR('${cr.id}')">
                  ✓ Sign &amp; Approve (Ed25519)
                </button>
                <button id="btn-request-changes" class="gh-btn gh-btn-warning" style="justify-content: center;" onclick="requestChanges('${cr.id}')">
                  📝 Request Changes
                </button>
                <button id="btn-reject-cr" class="gh-btn gh-btn-danger" style="justify-content: center;" onclick="rejectCR('${cr.id}')">
                  ✗ Reject Change Request
                </button>
              </div>
            `}
          </div>
        </div>
      </div>
    </div>

    <script>
      async function approveCR(crId) {
        const rationale = document.getElementById('pm-feedback-input')?.value.trim() || prompt("Enter human supervisor rationale for approval:", "PM verified live preview and real QA evidence");
        if (!rationale) return;

        const btn = document.getElementById('btn-approve-cr');
        if (btn) { btn.disabled = true; btn.textContent = '⏳ Signing (Ed25519)...'; }

        try {
          const { keyPair, pubHex } = await getOrGenerateReviewerKey();
          const nowSecs = Math.floor(Date.now() / 1000);

          const enc = new TextEncoder();
          const magic = enc.encode("S3ACR_APPROVE1");
          const tsBytes = new Uint8Array(8);
          new DataView(tsBytes.buffer).setBigUint64(0, BigInt(nowSecs), true);
          const crBytes = enc.encode(crId);
          const ratBytes = enc.encode(rationale);

          const totalLen = 14 + 8 + crBytes.length + 1 + ratBytes.length;
          const envBytes = new Uint8Array(totalLen);
          let off = 0;
          envBytes.set(magic, off); off += 14;
          envBytes.set(tsBytes, off); off += 8;
          envBytes.set(crBytes, off); off += crBytes.length;
          envBytes[off] = 0; off += 1;
          envBytes.set(ratBytes, off);

          const sig = await window.crypto.subtle.sign({ name: "Ed25519" }, keyPair.privateKey, envBytes);
          const sigHex = Array.from(new Uint8Array(sig)).map(b => b.toString(16).padStart(2, '0')).join('');

          const headers = getAdminAuthHeaders();
          const res = await fetch('/project/${projectId}/change-requests/' + crId + '/approve', {
            method: 'POST',
            headers: headers,
            body: JSON.stringify({
              reviewer_pubkey: pubHex,
              signature: sigHex,
              timestamp: nowSecs,
              rationale: rationale,
            })
          });
          if (res.ok) {
            alert("Change request approved with verified Ed25519 human signature and merged into baseline!");
            location.reload();
          } else if (res.status === 401) {
            if (btn) { btn.disabled = false; btn.textContent = '✓ Sign & Approve (Ed25519)'; }
            const sec = await promptAndStoreAdminSecret();
            if (sec) approveCR(crId);
          } else if (res.status === 403) {
            alert("Forbidden: Your browser public key is not enrolled in SUPERVISOR_PUBKEYS.\\nClick '🔑 Enroll Key' in the header to view and register your key.");
            if (btn) { btn.disabled = false; btn.textContent = '✓ Sign & Approve (Ed25519)'; }
          } else {
            const err = await res.json();
            alert("Approval failed: " + (err.error || JSON.stringify(err)));
            if (btn) { btn.disabled = false; btn.textContent = '✓ Sign & Approve (Ed25519)'; }
          }
        } catch (err) {
          alert("Cryptographic signing error: " + err.message);
          if (btn) { btn.disabled = false; btn.textContent = '✓ Sign & Approve (Ed25519)'; }
        }
      }

      async function requestChanges(crId) {
        const feedback = document.getElementById('pm-feedback-input')?.value.trim();
        if (!feedback) {
          alert("Please enter feedback notes describing the requested revision.");
          return;
        }
        const btn = document.getElementById('btn-request-changes');
        if (btn) { btn.disabled = true; btn.textContent = '⏳ Submitting...'; }

        try {
          const { pubHex } = await getOrGenerateReviewerKey();
          const res = await fetch('/project/${projectId}/change-requests/' + crId + '/request-changes', {
            method: 'POST',
            headers: getAdminAuthHeaders(),
            body: JSON.stringify({ feedback, reviewer_pubkey: pubHex })
          });
          if (res.ok) {
            alert("Changes requested! Feedback sent back to agent task.");
            location.reload();
          } else if (res.status === 401) {
            if (btn) { btn.disabled = false; btn.textContent = '📝 Request Changes'; }
            const sec = await promptAndStoreAdminSecret();
            if (sec) requestChanges(crId);
          } else {
            alert("Request failed: " + await res.text());
            if (btn) { btn.disabled = false; btn.textContent = '📝 Request Changes'; }
          }
        } catch (err) {
          alert("Supervisor key error: " + err.message);
          if (btn) { btn.disabled = false; btn.textContent = '📝 Request Changes'; }
        }
      }

      async function rejectCR(crId) {
        const rationale = document.getElementById('pm-feedback-input')?.value.trim() || "Rejected by PM supervisor";
        const btn = document.getElementById('btn-reject-cr');
        if (btn) { btn.disabled = true; btn.textContent = '⏳ Rejecting...'; }

        try {
          const { pubHex } = await getOrGenerateReviewerKey();
          const res = await fetch('/project/${projectId}/change-requests/' + crId + '/reject', {
            method: 'POST',
            headers: getAdminAuthHeaders(),
            body: JSON.stringify({ rationale, reviewer_pubkey: pubHex })
          });
          if (res.ok) {
            alert("Change request rejected.");
            location.reload();
          } else if (res.status === 401) {
            if (btn) { btn.disabled = false; btn.textContent = '✗ Reject Change Request'; }
            const sec = await promptAndStoreAdminSecret();
            if (sec) rejectCR(crId);
          } else {
            alert("Rejection failed: " + await res.text());
            if (btn) { btn.disabled = false; btn.textContent = '✗ Reject Change Request'; }
          }
        } catch (err) {
          alert("Supervisor key error: " + err.message);
          if (btn) { btn.disabled = false; btn.textContent = '✗ Reject Change Request'; }
        }
      }
    </script>
  `;
}
