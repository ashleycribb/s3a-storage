// cloudflare/src/ui/pages/insights.ts
// Insights Page: s3a-ledger Budget Agent & s3a-trace Fine-Tuning Replay (/p/:id/insights)

import type { BudgetStatus, TaskItem } from "../../types";

export interface InsightsPageOptions {
  projectId: string;
  budgetData?: BudgetStatus;
  tasks?: TaskItem[];
}

export function renderInsightsPage(options: InsightsPageOptions): string {
  const { projectId, budgetData, tasks = [] } = options;

  const spentToday = budgetData?.spent_today || 0.0;
  const cap = budgetData?.cap || 5.0;
  const pct = budgetData?.pct_spent || 0.0;
  const isWarn = budgetData?.warning_80_pct || false;
  const isCapped = budgetData?.cap_reached || false;
  const timeResetSecs = budgetData?.time_until_reset_secs || 86400;
  const hoursLeft = Math.floor(timeResetSecs / 3600);
  const minsLeft = Math.floor((timeResetSecs % 3600) / 60);

  const deferredQueue = budgetData?.deferred_queue || [];
  const entries = budgetData?.entries || [];

  return `
    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1.5rem; gap: 1rem; flex-wrap: wrap;">
      <div>
        <h2 style="font-size: 1.25rem; font-weight: 600; color: var(--gh-text-heading); margin-bottom: 0.2rem;">
          Platform Insights &bull; Cost &amp; Trace Telemetry
        </h2>
        <p style="color: var(--gh-text-muted); font-size: 13px;">
          Autonomous compute budgeting, spend guardrails, and fine-tuning harness replay.
        </p>
      </div>
    </div>

    <!-- s3a-ledger Budget Agent Card -->
    <div class="gh-card" style="margin-bottom: 1.5rem; border-color: ${isCapped ? 'var(--gh-red)' : isWarn ? 'var(--gh-yellow)' : 'var(--gh-border)'};">
      <div class="gh-card-header" style="background: #161b22;">
        <div style="display: flex; align-items: center; gap: 0.5rem;">
          <strong style="color: var(--gh-text-heading); font-size: 13px;">s3a-ledger &bull; Budget Agent</strong>
          ${isCapped
            ? '<span class="gh-badge gh-badge-danger">100% Cap Reached (Paid Steps Deferred)</span>'
            : isWarn
            ? '<span class="gh-badge gh-badge-yellow">⚠️ 80% Cap Warning</span>'
            : '<span class="gh-badge gh-badge-success">Budget Nominal</span>'}
        </div>
        <div style="display: flex; gap: 0.5rem;">
          <button class="gh-btn gh-btn-sm" onclick="configureBudgetCap()">⚙️ Configure Cap</button>
          <button class="gh-btn gh-btn-sm" onclick="resetBudgetToday()">🔄 Reset Today</button>
        </div>
      </div>

      <div class="gh-card-body">
        <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 1.5rem; margin-bottom: 1.25rem;">
          <div>
            <span style="color: var(--gh-text-muted); font-size: 12px;">Spent Today</span>
            <div style="font-size: 1.5rem; font-weight: 700; color: ${isCapped ? 'var(--gh-red-text)' : 'var(--gh-text-heading)'};">
              $${spentToday.toFixed(5)}
            </div>
            <span style="font-size: 11px; color: var(--gh-text-muted);">Reset at 00:00 UTC</span>
          </div>

          <div>
            <span style="color: var(--gh-text-muted); font-size: 12px;">Daily Cap</span>
            <div style="font-size: 1.5rem; font-weight: 700; color: var(--gh-text-heading);">
              $${cap.toFixed(2)}
            </div>
            <span style="font-size: 11px; color: var(--gh-text-muted);">${pct.toFixed(1)}% consumed</span>
          </div>

          <div>
            <span style="color: var(--gh-text-muted); font-size: 12px;">Time Until Reset</span>
            <div style="font-size: 1.5rem; font-weight: 700; color: #38bdf8;">
              ${hoursLeft}h ${minsLeft}m
            </div>
            <span style="font-size: 11px; color: var(--gh-text-muted);">Resets daily cap to $0.00</span>
          </div>

          <div>
            <span style="color: var(--gh-text-muted); font-size: 12px;">Deferred Queue</span>
            <div style="font-size: 1.5rem; font-weight: 700; color: ${deferredQueue.length > 0 ? 'var(--gh-yellow-text)' : 'var(--gh-green-text)'};">
              ${deferredQueue.length}
            </div>
            <span style="font-size: 11px; color: var(--gh-text-muted);">${deferredQueue.length > 0 ? 'Queued until reset' : 'Free work active'}</span>
          </div>
        </div>

        <!-- Spend Progress Bar -->
        <div style="background: #21262d; border-radius: 4px; height: 8px; width: 100%; overflow: hidden; margin-bottom: 1.25rem;">
          <div style="background: ${isCapped ? 'var(--gh-red)' : isWarn ? 'var(--gh-yellow)' : 'var(--gh-green)'}; height: 100%; width: ${Math.min(pct, 100)}%;"></div>
        </div>

        <!-- Deferred Queue Details -->
        ${deferredQueue.length > 0 ? `
          <div style="background: rgba(210,153,34,0.1); border: 1px solid rgba(210,153,34,0.3); border-radius: 6px; padding: 0.85rem; margin-bottom: 1.25rem;">
            <strong style="color: var(--gh-yellow-text); font-size: 12px; display: block; margin-bottom: 0.4rem;">Queued Deferred Actions (Paid steps paused, free work active):</strong>
            <ul style="padding-left: 1.2rem; font-size: 12px; color: var(--gh-text);">
              ${deferredQueue.map(d => `
                <li>Task: <code>${d.task_id}</code> &bull; Agent: <code>${d.agent_id}</code> &bull; Action: <code>${d.kind}</code> (${d.units} units, est $${d.est_cost.toFixed(5)})</li>
              `).join('')}
            </ul>
          </div>
        ` : ''}

        <!-- Recent Ledger Spend Log -->
        <strong style="font-size: 12px; color: var(--gh-text-heading); display: block; margin-bottom: 0.5rem;">Recent Recorded Paid Actions:</strong>
        <div style="max-height: 200px; overflow-y: auto; background: #0d1117; border: 1px solid var(--gh-border); border-radius: 6px;">
          ${entries.length === 0
            ? '<div style="padding: 1rem; color: var(--gh-text-muted); font-size: 12px; text-align: center;">No paid actions recorded yet today.</div>'
            : entries.map(e => `
              <div style="display: flex; justify-content: space-between; padding: 0.4rem 0.75rem; border-bottom: 1px solid var(--gh-border-subtle); font-size: 12px;">
                <span><code>${new Date(e.ts).toLocaleTimeString()}</code> &bull; Task: <code>${e.task_id}</code> &bull; Agent: <code>${e.agent_id}</code> &bull; Kind: <span class="gh-badge gh-badge-muted">${e.kind}</span></span>
                <span style="font-family: monospace; color: ${e.is_free ? 'var(--gh-text-muted)' : 'var(--gh-green-text)'};">${e.is_free ? 'FREE ($0.00)' : `$${e.est_cost.toFixed(5)}`}</span>
              </div>
            `).join('')}
        </div>
      </div>
    </div>

    <!-- Code Health Review Findings (TASK-283) -->
    <div class="gh-card" style="margin-bottom: 1.5rem;">
      <div class="gh-card-header" style="background: #161b22; display: flex; justify-content: space-between; align-items: center;">
        <div style="display: flex; align-items: center; gap: 0.5rem;">
          <strong style="color: var(--gh-text-heading); font-size: 13px;">🔍 Code Health Review &bull; TASK-283 Findings</strong>
          <span class="gh-badge gh-badge-purple">Read-Only Review</span>
        </div>
        <div style="font-size: 12px; color: var(--gh-text-muted);">
          Findings grouped by severity &bull; Human (Ashley + Ward) Triage
        </div>
      </div>
      <div class="gh-card-body">
        <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 1rem; margin-bottom: 1.25rem;">
          <div style="background: rgba(248,81,73,0.1); border: 1px solid rgba(248,81,73,0.3); border-radius: 6px; padding: 0.85rem;">
            <span style="color: var(--gh-red-text); font-size: 12px; font-weight: 600;">High Severity</span>
            <div style="font-size: 1.3rem; font-weight: 700; color: var(--gh-red-text); margin-top: 0.25rem;">2</div>
            <span style="font-size: 11px; color: var(--gh-text-muted);">Security smell, Draw validation</span>
          </div>
          <div style="background: rgba(210,153,34,0.1); border: 1px solid rgba(210,153,34,0.3); border-radius: 6px; padding: 0.85rem;">
            <span style="color: var(--gh-yellow-text); font-size: 12px; font-weight: 600;">Medium Severity</span>
            <div style="font-size: 1.3rem; font-weight: 700; color: var(--gh-yellow-text); margin-top: 0.25rem;">3</div>
            <span style="font-size: 11px; color: var(--gh-text-muted);">Linear scan, A11y draw rule, Duplicate team</span>
          </div>
          <div style="background: rgba(88,166,255,0.1); border: 1px solid rgba(88,166,255,0.3); border-radius: 6px; padding: 0.85rem;">
            <span style="color: #58a6ff; font-size: 12px; font-weight: 600;">Low Severity</span>
            <div style="font-size: 1.3rem; font-weight: 700; color: #58a6ff; margin-top: 0.25rem;">2</div>
            <span style="font-size: 11px; color: var(--gh-text-muted);">Dead error copy, Inverted leaf check</span>
          </div>
          <div style="background: #21262d; border-radius: 6px; padding: 0.85rem; border: 1px solid var(--gh-border);">
            <span style="color: var(--gh-text-muted); font-size: 12px; font-weight: 600;">Total Findings</span>
            <div style="font-size: 1.3rem; font-weight: 700; color: var(--gh-text-heading); margin-top: 0.25rem;">7</div>
            <span style="font-size: 11px; color: var(--gh-green-text);">0 diffs &bull; 0 commits &bull; 0 CRs</span>
          </div>
        </div>

        <div style="display: flex; flex-direction: column; gap: 0.65rem;">
          <div style="padding: 0.75rem; background: #0d1117; border-radius: 6px; border: 1px solid var(--gh-border); border-left: 3px solid var(--gh-red);">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.25rem;">
              <strong style="color: var(--gh-text-heading); font-size: 13px;">Security: Missing cryptographic signature validation in validateEscrowRelease</strong>
              <span class="gh-badge gh-badge-danger">HIGH SEVERITY</span>
            </div>
            <div style="font-family: monospace; font-size: 12px; color: var(--gh-orange); margin-bottom: 0.35rem;">demo/brakets-bracket/lib/escrow.ts:12</div>
            <p style="font-size: 12.5px; color: var(--gh-text-muted); margin: 0;">validateEscrowRelease only verifies array length against multiSigThreshold without cryptographically verifying Ed25519 signatures against signersRequired list. Flag only: never auto-fix cashier/escrow/payout.</p>
          </div>

          <div style="padding: 0.75rem; background: #0d1117; border-radius: 6px; border: 1px solid var(--gh-border); border-left: 3px solid var(--gh-red);">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.25rem;">
              <strong style="color: var(--gh-text-heading); font-size: 13px;">Bug: Draw score validation scoped only to single_elim format</strong>
              <span class="gh-badge gh-badge-danger">HIGH SEVERITY</span>
            </div>
            <div style="font-family: monospace; font-size: 12px; color: var(--gh-orange); margin-bottom: 0.35rem;">demo/brakets-bracket/packages/core/src/tournament.ts:138</div>
            <p style="font-size: 12.5px; color: var(--gh-text-muted); margin: 0;">validateMatchScore checks scores[0] === scores[1] only when bracketType === 'single_elim'. Draws in double-elimination lower brackets and playoffs remain unhandled.</p>
          </div>

          <div style="padding: 0.75rem; background: #0d1117; border-radius: 6px; border: 1px solid var(--gh-border); border-left: 3px solid var(--gh-yellow);">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.25rem;">
              <strong style="color: var(--gh-text-heading); font-size: 13px;">Perf / Slow Query: Hardcoded 4-team lobby capacity and linear array search in match advancement</strong>
              <span class="gh-badge gh-badge-yellow">MEDIUM SEVERITY</span>
            </div>
            <div style="font-family: monospace; font-size: 12px; color: var(--gh-orange); margin-bottom: 0.35rem;">demo/brakets-bracket/lib/mock-api.ts:102</div>
            <p style="font-size: 12.5px; color: var(--gh-text-muted); margin: 0;">Match advancement performs linear find() over lobbies and hardcodes teamIds.length &lt; 4, causing O(N) lookup bottleneck and failing for variable-size lobbies.</p>
          </div>

          <div style="padding: 0.75rem; background: #0d1117; border-radius: 6px; border: 1px solid var(--gh-border); border-left: 3px solid var(--gh-yellow);">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.25rem;">
              <strong style="color: var(--gh-text-heading); font-size: 13px;">A11y / UI Bug: MatchScoreForm hardcodes draw rejection across all formats</strong>
              <span class="gh-badge gh-badge-yellow">MEDIUM SEVERITY</span>
            </div>
            <div style="font-family: monospace; font-size: 12px; color: var(--gh-orange); margin-bottom: 0.35rem;">demo/brakets-bracket/components/MatchScoreForm.tsx:31</div>
            <p style="font-size: 12.5px; color: var(--gh-text-muted); margin: 0;">Client-side validation rule score1 !== score2 unconditionally rejects equal scores, disabling valid score reporting for round-robin and swiss formats and lacking accessible error messaging.</p>
          </div>

          <div style="padding: 0.75rem; background: #0d1117; border-radius: 6px; border: 1px solid var(--gh-border); border-left: 3px solid var(--gh-yellow);">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.25rem;">
              <strong style="color: var(--gh-text-heading); font-size: 13px;">Missing Tests / Validation: LobbyReport standings schema allows duplicate team IDs</strong>
              <span class="gh-badge gh-badge-yellow">MEDIUM SEVERITY</span>
            </div>
            <div style="font-family: monospace; font-size: 12px; color: var(--gh-orange); margin-bottom: 0.35rem;">demo/brakets-bracket/packages/core/src/finals.ts:29</div>
            <p style="font-size: 12.5px; color: var(--gh-text-muted); margin: 0;">LobbyReport.standings zod schema specifies z.array(RosterId).min(2).max(4) without checking for duplicate team entries, allowing malformed reports.</p>
          </div>

          <div style="padding: 0.75rem; background: #0d1117; border-radius: 6px; border: 1px solid var(--gh-border); border-left: 3px solid #58a6ff;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.25rem;">
              <strong style="color: var(--gh-text-heading); font-size: 13px;">Dead Code: Hardcoded 8-team roster validation error copy</strong>
              <span class="gh-badge gh-badge-muted">LOW SEVERITY</span>
            </div>
            <div style="font-family: monospace; font-size: 12px; color: var(--gh-orange); margin-bottom: 0.35rem;">demo/brakets-bracket/lib/create-errors.ts:8</div>
            <p style="font-size: 12.5px; color: var(--gh-text-muted); margin: 0;">CREATE_ERROR_COPY hardcodes eight_team_names_required and roster_need_eight strings, preventing generic error messages for 4-team, 16-team, or custom bracket sizes.</p>
          </div>

          <div style="padding: 0.75rem; background: #0d1117; border-radius: 6px; border: 1px solid var(--gh-border); border-left: 3px solid #58a6ff;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.25rem;">
              <strong style="color: var(--gh-text-heading); font-size: 13px;">Duplicated Logic / Bug: Inverted leaf node check misclassifies tournament round numbers</strong>
              <span class="gh-badge gh-badge-muted">LOW SEVERITY</span>
            </div>
            <div style="font-family: monospace; font-size: 12px; color: var(--gh-orange); margin-bottom: 0.35rem;">demo/brakets-bracket/components/MatchCard.tsx:49</div>
            <p style="font-size: 12.5px; color: var(--gh-text-muted); margin: 0;">MatchCard determines round 1 via match.children &amp;&amp; match.children.length === 0, inverting hierarchical tree traversal where root nodes represent finals.</p>
          </div>
        </div>
      </div>
    </div>

    <!-- s3a-trace Fine-Tuning Replay Card -->
    <div class="gh-card">
      <div class="gh-card-header" style="background: #161b22;">
        <strong style="color: var(--gh-text-heading); font-size: 13px;">s3a-trace &bull; Replay Timeline &amp; Fine-Tuning Export</strong>
        <span style="font-size: 12px; color: var(--gh-text-muted);">xAPI Verbs &bull; Append-Only S3A Record</span>
      </div>
      <div class="gh-card-body">
        <p style="font-size: 13px; color: var(--gh-text-muted); margin-bottom: 1rem;">
          Every workflow step (prompt, model, patch, build/test log, QA evidence, human decision) is an append-only S3A record with human vs AI provenance.
        </p>

        <div style="display: flex; gap: 0.5rem; align-items: center; margin-bottom: 1rem; flex-wrap: wrap;">
          <select id="trace-task-select" class="gh-select" style="max-width: 320px;">
            ${tasks.map(t => `<option value="${t.id}">${t.id}: ${t.title}</option>`).join('')}
            ${tasks.length === 0 ? '<option value="">No tasks available</option>' : ''}
          </select>
          <button class="gh-btn" onclick="openSelectedTrace()">
            ▶ Step Through Timeline
          </button>
          <button class="gh-btn" onclick="exportSelectedTraceJsonl()">
            📥 Export to Harness JSONL
          </button>
        </div>

        <div id="inline-trace-container" style="background: #0d1117; border: 1px solid var(--gh-border); border-radius: 6px; padding: 1rem; min-height: 120px;">
          <div style="color: var(--gh-text-muted); font-size: 13px; text-align: center; padding: 1.5rem 0;">
            Select a task above and click "Step Through Timeline" to replay the xAPI provenance steps.
          </div>
        </div>
      </div>
    </div>

    <!-- Trace Replay Modal -->
    <div id="trace-modal" class="modal-overlay">
      <div style="background: #161b22; border: 1px solid var(--gh-border); border-radius: 8px; width: 100%; max-width: 750px; max-height: 85vh; display: flex; flex-direction: column; overflow: hidden; box-shadow: 0 20px 40px rgba(0,0,0,0.85);">
        <div style="padding: 1rem 1.25rem; border-bottom: 1px solid var(--gh-border); display: flex; justify-content: space-between; align-items: center; background: #0d1117;">
          <strong id="trace-modal-title" style="color: var(--gh-text-heading); font-size: 1.05rem;">Task Trace Replay</strong>
          <button onclick="closeTraceModal()" style="background: transparent; border: none; color: var(--gh-text-muted); font-size: 1.2rem; cursor: pointer;">✕</button>
        </div>
        <div id="trace-steps-container" style="padding: 1.25rem; overflow-y: auto; flex: 1;"></div>
      </div>
    </div>

    <script>
      function closeTraceModal() {
        const modal = document.getElementById("trace-modal");
        if (modal) modal.style.display = "none";
      }

      function openSelectedTrace() {
        const sel = document.getElementById("trace-task-select");
        if (sel && sel.value) openTraceModal(sel.value);
      }

      async function exportSelectedTraceJsonl() {
        const sel = document.getElementById("trace-task-select");
        if (!sel || !sel.value) return;
        try {
          const res = await fetch("/project/${projectId}/trace/" + sel.value + "/export-jsonl");
          if (!res.ok) {
            alert("No training data samples found in trace.");
            return;
          }
          const text = await res.text();
          const blob = new Blob([text], { type: "application/jsonl" });
          const url = URL.createObjectURL(blob);
          const a = document.createElement("a");
          a.href = url;
          a.download = "s3a-trace-" + sel.value + ".jsonl";
          a.click();
        } catch (e) {
          alert("Export error: " + e.message);
        }
      }

      async function configureBudgetCap() {
        const currentCap = "${cap}";
        const val = prompt("Enter daily compute budget cap in USD ($):", currentCap);
        if (!val || isNaN(parseFloat(val))) return;
        try {
          const res = await fetch("/project/${projectId}/ledger/config", {
            method: "POST",
            headers: getAdminAuthHeaders(),
            body: JSON.stringify({ daily_cap: parseFloat(val) })
          });
          if (res.ok) {
            alert("Daily budget cap updated to $" + parseFloat(val).toFixed(4));
            location.reload();
          } else if (res.status === 401) {
            const sec = await promptAndStoreAdminSecret();
            if (sec) configureBudgetCap();
          } else {
            alert("Error updating budget cap: " + await res.text());
          }
        } catch (e) {
          alert("Error: " + e.message);
        }
      }

      async function resetBudgetToday() {
        if (!confirm("Reset today's spent counter and resume all deferred tasks?")) return;
        try {
          const res = await fetch("/project/${projectId}/ledger/reset", {
            method: "POST",
            headers: getAdminAuthHeaders(),
          });
          if (res.ok) {
            const data = await res.json();
            alert("Budget window reset. Resumed " + (data.resumed_count || 0) + " deferred task(s).");
            location.reload();
          } else if (res.status === 401) {
            const sec = await promptAndStoreAdminSecret();
            if (sec) resetBudgetToday();
          } else {
            alert("Error resetting budget: " + await res.text());
          }
        } catch (e) {
          alert("Error: " + e.message);
        }
      }

      async function openTraceModal(taskId) {
        const modal = document.getElementById("trace-modal");
        const title = document.getElementById("trace-modal-title");
        const container = document.getElementById("trace-steps-container");
        if (!modal || !container) return;

        modal.style.display = "flex";
        title.textContent = "Task Trace Replay: " + taskId;
        container.innerHTML = '<div style="color: var(--gh-text-muted); padding: 1rem;">Loading immutable S3A trace timeline...</div>';

        try {
          const res = await fetch("/project/${projectId}/trace/" + taskId);
          if (!res.ok) {
            container.innerHTML = '<div style="color: #fca5a5; padding: 1rem;">No trace records recorded yet for task ' + taskId + '. Execute an autonomous step to generate provenance.</div>';
            return;
          }
          const trace = await res.json();
          if (!trace.steps || trace.steps.length === 0) {
            container.innerHTML = '<div style="color: var(--gh-text-muted); padding: 1rem;">No trace steps recorded for this task.</div>';
            return;
          }

          let h = '<div style="display: flex; flex-direction: column; gap: 0.75rem;">';
          for (const s of trace.steps) {
            const isAi = s.actor.type === "ai";
            const actorBadge = isAi
              ? '<span class="gh-badge gh-badge-blue">🤖 AI (' + (s.actor.name || s.actor.id) + ')</span>'
              : '<span class="gh-badge gh-badge-success">👤 HUMAN (' + (s.actor.name || s.actor.id) + ')</span>';

            const verbLabel = (s.xapi_verb || "").split("/").pop() || s.xapi_verb;
            const timeStr = new Date(s.ts).toLocaleTimeString();

            h += '<div style="background: #0d1117; border: 1px solid var(--gh-border); border-radius: 6px; padding: 0.75rem;">';
            h += '  <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.35rem;">';
            h += '    <div style="display: flex; align-items: center; gap: 0.4rem;">';
            h += '      <strong style="color: #f0f6fc; font-size: 0.85rem;">' + s.step_name + '</strong>';
            h += '      <span class="gh-badge gh-badge-muted" style="font-family: monospace; font-size: 0.7rem;">' + verbLabel + '</span>';
            h += '    </div>';
            h += '    <span style="font-size: 0.75rem; color: var(--gh-text-muted);">' + timeStr + '</span>';
            h += '  </div>';
            h += '  <div style="margin-bottom: 0.35rem;">' + actorBadge + '</div>';
            if (s.evidence) {
              h += '  <div style="font-size: 0.78rem; color: var(--gh-text-muted); font-family: monospace; margin-top: 0.25rem;">';
              h += '    Evidence: ' + (s.evidence.test_stage || s.evidence.kind || 'verified') + ' &bull; Hash: ' + (s.evidence.seal_hash || s.evidence.diff_hash || '').slice(0, 16) + '...';
              h += '  </div>';
            }
            if (s.human_decision) {
              h += '  <div style="margin-top: 0.35rem; font-size: 0.78rem; color: #fde68a;">';
              h += '    <strong>Human Decision:</strong> Rationale: <em>"' + (s.human_decision.rationale || '') + '"</em> &bull; Key: <code>' + (s.human_decision.reviewer_pubkey || '').slice(0, 16) + '...</code>';
              h += '  </div>';
            }
            h += '</div>';
          }
          h += '</div>';
          container.innerHTML = h;
        } catch (err) {
          container.innerHTML = '<div style="color: #fca5a5; padding: 1rem;">Failed to load trace: ' + err.message + '</div>';
        }
      }
    </script>
  `;
}
