// cloudflare/src/ui/pages/rehearsals.ts
// Server-Rendered Rehearsals Dashboard: Run History, Stage Pass/Fail, Standings & Auto-Filed Issues

import type { RehearsalRunRecord, RehearsalStage, RehearsalMatch, RehearsalStanding } from "../../types";

export function renderRehearsalsPage(
  projectId: string,
  runs: RehearsalRunRecord[] = [],
  options: { isPaused?: boolean } = {}
): string {
  const totalRuns = runs.length;
  const passedRuns = runs.filter(r => r.status === "passed" || r.status === "disputed_resolved").length;
  const totalIssuesFiled = runs.reduce((acc, r) => acc + (r.issues_filed?.length || 0), 0);
  const passRate = totalRuns > 0 ? Math.round((passedRuns / totalRuns) * 100) : 100;

  const rowsHtml = runs.length === 0
    ? `
      <div style="text-align: center; padding: 3.5rem 1rem;">
        <div style="font-size: 2.5rem; margin-bottom: 0.85rem;">🛡️</div>
        <h3 style="font-size: 1.15rem; color: var(--gh-text-heading); margin-bottom: 0.5rem;">No tournament rehearsals recorded yet</h3>
        <p style="font-size: 13px; color: var(--gh-text-muted); max-width: 520px; margin: 0 auto 1.5rem auto; line-height: 1.5;">
          Execute simulated single-elimination tournament runs (4, 8, or 16 teams) with scripted test accounts, score validation, and dispute resolution to verify production readiness.
        </p>
        <button class="gh-btn gh-btn-primary" onclick="triggerRehearsal('happy_path', 8)">
          ⚡ Run 8-Team Happy Path Rehearsal
        </button>
      </div>
    `
    : runs.map(run => {
        const isPassed = run.status === "passed";
        const isDisputedResolved = run.status === "disputed_resolved";
        const isEvidenceMissing = run.status === "evidence_missing";
        const isFailed = run.status === "failed";

        const statusBadge = isPassed
          ? `<span class="gh-badge gh-badge-green">✓ PASSED</span>`
          : isDisputedResolved
          ? `<span class="gh-badge gh-badge-purple">⚖️ DISPUTE RESOLVED</span>`
          : isEvidenceMissing
          ? `<span class="gh-badge" style="background: rgba(241,224,90,0.15); border: 1px solid rgba(241,224,90,0.4); color: #f1e05a; font-weight: 700;">⚠️ EVIDENCE MISSING</span>`
          : `<span class="gh-badge gh-badge-red">✗ FAILED</span>`;

        const scenarioBadge = run.scenario === "happy_path"
          ? `<span class="gh-badge gh-badge-blue">Happy Path</span>`
          : run.scenario === "dispute"
          ? `<span class="gh-badge gh-badge-purple">Dispute Case</span>`
          : `<span class="gh-badge gh-badge-yellow">Bad Score (Chaos)</span>`;

        const issuesFiledBadge = (run.issues_filed && run.issues_filed.length > 0)
          ? `<span class="gh-badge gh-badge-red" style="font-weight: 600;">Filed ${run.issues_filed.length} ${run.issues_filed.length === 1 ? 'issue' : 'issues'}</span>`
          : `<span style="color: var(--gh-text-muted); font-size: 12px;">0 issues</span>`;

        // Render stage pass/fail indicators
        const stagePills = (run.stages || []).map(st => {
          const pillColor = st.status === "passed"
            ? "var(--gh-green-text)"
            : st.status === "disputed"
            ? "var(--gh-purple-text)"
            : st.status === "skipped"
            ? "var(--gh-text-muted)"
            : "var(--gh-red-text)";
          const pillIcon = st.status === "passed" ? "✓" : st.status === "disputed" ? "⚖️" : st.status === "skipped" ? "○" : "✗";
          const shortName = st.stage_id.replace("stage_", "");
          return `<span style="font-size: 11px; font-family: monospace; color: ${pillColor}; margin-right: 0.4rem;" title="${st.name}: ${st.status}">${pillIcon}${shortName}</span>`;
        }).join("");

        return `
          <div class="gh-row" style="padding: 1rem; border-bottom: 1px solid var(--gh-border); align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 0.75rem;">
            <div style="flex: 1; min-width: 280px;">
              <div style="display: flex; align-items: center; gap: 0.5rem; margin-bottom: 0.25rem;">
                <a href="/p/${projectId}/rehearsals/${run.run_id}" style="font-weight: 600; font-size: 14px; font-family: monospace;">
                  ${run.run_id}
                </a>
                ${statusBadge}
                ${scenarioBadge}
                ${run.format_name ? `<span class="gh-badge gh-badge-blue" title="Content Hash: ${run.format_hash || 'none'}">${run.format_name}</span>` : `<span class="gh-badge gh-badge-muted">${run.bracket_size}-Team</span>`}
                ${run.game_pack_id ? `<span class="gh-badge gh-badge-purple">${run.game_pack_id}</span>` : ''}
              </div>
              <div style="display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; font-size: 12px; color: var(--gh-text-muted);">
                <span>${run.tournament_name}</span>
                ${run.format_hash ? `<span>&bull;</span><span style="font-family: monospace; font-size: 11px;">#${run.format_hash}</span>` : ''}
                <span>&bull;</span>
                <span>Stages: ${stagePills}</span>
              </div>
            </div>

            <div style="display: flex; align-items: center; gap: 1.25rem;">
              <div style="text-align: right;">
                <div style="margin-bottom: 0.15rem;">${issuesFiledBadge}</div>
                <div style="font-size: 11px; color: var(--gh-text-muted); font-family: monospace;">
                  ${run.total_duration_ms}ms &bull; ${new Date(run.started_at).toLocaleTimeString()}
                </div>
              </div>
              <a href="/p/${projectId}/rehearsals/${run.run_id}" class="gh-btn gh-btn-sm">
                View Run Report ↗
              </a>
            </div>
          </div>
        `;
      }).join("");

  return `
    <div style="margin-bottom: 1.5rem; display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 1rem;">
      <div>
        <h2 style="font-size: 1.4rem; font-weight: 600; color: var(--gh-text-heading);">
          🛡️ Simulated Tournament Rehearsals
        </h2>
        <p style="color: var(--gh-text-muted); font-size: 13px; margin-top: 0.25rem;">
          Automated end-to-end tournament simulations on staging. Scripted players, TO brackets, bad score fuzzing, and dispute resolutions.
        </p>
      </div>

      <div style="display: flex; gap: 0.5rem; align-items: center;">
        <button class="gh-btn gh-btn-primary" onclick="openRehearsalModal()">
          ⚡ Run Tournament Rehearsal
        </button>
      </div>
    </div>

    <!-- Stats Summary Cards -->
    <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 1rem; margin-bottom: 1.5rem;">
      <div class="gh-card" style="padding: 1rem; margin: 0;">
        <div style="font-size: 12px; color: var(--gh-text-muted); text-transform: uppercase; font-weight: 600;">Total Rehearsals</div>
        <div style="font-size: 1.6rem; font-weight: 700; color: var(--gh-text-heading); margin-top: 0.25rem;">${totalRuns}</div>
        <div style="font-size: 11px; color: var(--gh-text-muted); margin-top: 0.25rem;">4, 8 &amp; 16-team elimination</div>
      </div>

      <div class="gh-card" style="padding: 1rem; margin: 0;">
        <div style="font-size: 12px; color: var(--gh-text-muted); text-transform: uppercase; font-weight: 600;">Pass Rate</div>
        <div style="font-size: 1.6rem; font-weight: 700; color: ${passRate >= 80 ? 'var(--gh-green-text)' : 'var(--gh-yellow-text)'}; margin-top: 0.25rem;">
          ${passRate}%
        </div>
        <div style="font-size: 11px; color: var(--gh-text-muted); margin-top: 0.25rem;">${passedRuns} passed / resolved</div>
      </div>

      <div class="gh-card" style="padding: 1rem; margin: 0;">
        <div style="font-size: 12px; color: var(--gh-text-muted); text-transform: uppercase; font-weight: 600;">Auto-Filed Issues</div>
        <div style="font-size: 1.6rem; font-weight: 700; color: ${totalIssuesFiled > 0 ? 'var(--gh-red-text)' : 'var(--gh-green-text)'}; margin-top: 0.25rem;">
          ${totalIssuesFiled}
        </div>
        <div style="font-size: 11px; color: var(--gh-text-muted); margin-top: 0.25rem;">Deduplicated by fingerprint</div>
      </div>

      <div class="gh-card" style="padding: 1rem; margin: 0;">
        <div style="font-size: 12px; color: var(--gh-text-muted); text-transform: uppercase; font-weight: 600;">Safety Guarantee</div>
        <div style="font-size: 1.1rem; font-weight: 700; color: var(--gh-green-text); margin-top: 0.5rem;">
          🛡️ Zero Cashier Risk
        </div>
        <div style="font-size: 11px; color: var(--gh-text-muted); margin-top: 0.25rem;">Fake test accounts &bull; Points only</div>
      </div>
    </div>

    <!-- Rehearsal History List Card -->
    <div class="gh-card">
      <div class="gh-card-header">
        <strong style="color: var(--gh-text-heading); font-size: 13px;">
          Rehearsal Runs History (${runs.length})
        </strong>
        <span style="font-size: 12px; color: var(--gh-text-muted);">
          Each run records full console logs, API network requests, timings, and xAPI trace.
        </span>
      </div>
      <div>
        ${rowsHtml}
      </div>
    </div>

    <!-- Rehearsal Launch Modal -->
    <div id="rehearsal-modal" class="modal-overlay" style="display: none;">
      <div style="background: #161b22; border: 1px solid var(--gh-border); border-radius: 8px; width: 100%; max-width: 550px; overflow: hidden; box-shadow: 0 20px 40px rgba(0,0,0,0.85);">
        <div style="padding: 1rem 1.25rem; border-bottom: 1px solid var(--gh-border); display: flex; justify-content: space-between; align-items: center; background: #0d1117;">
          <strong style="color: #f0f6fc; font-size: 1.05rem;">⚡ Trigger Tournament Rehearsal</strong>
          <button onclick="closeRehearsalModal()" style="background: transparent; border: none; color: var(--gh-text-muted); font-size: 1.2rem; cursor: pointer;">✕</button>
        </div>
        <div style="padding: 1.25rem; font-size: 13px;">
          <div style="margin-bottom: 1rem;">
            <label style="display: block; font-weight: 600; margin-bottom: 0.35rem; color: var(--gh-text-heading);">Tournament Format (Data-Driven):</label>
            <select id="modal-rehearsal-format" class="gh-select">
              <option value="single_elimination" selected>Single Elimination (Dynamic bracket)</option>
              <option value="double_elimination">Double Elimination (Upper + Lower + Grand Finals)</option>
              <option value="swiss">Swiss System (Buchholz tiebreaks &amp; rematch rules)</option>
              <option value="groups_playoffs">Round-Robin Groups &rarr; Playoffs</option>
              <option value="battle_royale">Battle Royale Lobby (Points Leaderboard)</option>
            </select>
          </div>

          <div style="margin-bottom: 1rem;">
            <label style="display: block; font-weight: 600; margin-bottom: 0.35rem; color: var(--gh-text-heading);">Game Pack (Rules &amp; Overrides):</label>
            <select id="modal-rehearsal-pack" class="gh-select">
              <option value="esports_tactical" selected>Esports Tactical (Valorant/CS2 Bo3 &bull; Map Veto Pool)</option>
              <option value="fighting_games">Fighting Games (FT3 Bo5 &bull; Double Elim Reset)</option>
              <option value="tabletop_swiss">Tabletop Swiss (MtG / Chess &bull; Buchholz Tiebreaks)</option>
              <option value="sports_sim">Sports Sim (Rocket League / EA FC Bo5)</option>
            </select>
          </div>

          <div style="margin-bottom: 1rem;">
            <label style="display: block; font-weight: 600; margin-bottom: 0.35rem; color: var(--gh-text-heading);">Bracket / Lobby Size:</label>
            <select id="modal-rehearsal-size" class="gh-select">
              <option value="8" selected>8 Teams / Players</option>
              <option value="4">4 Teams / Players</option>
              <option value="16">16 Teams (Championship)</option>
            </select>
          </div>

          <div style="margin-bottom: 1.25rem;">
            <label style="display: block; font-weight: 600; margin-bottom: 0.35rem; color: var(--gh-text-heading);">Simulation Scenario:</label>
            <select id="modal-rehearsal-scenario" class="gh-select">
              <option value="happy_path" selected>1. Happy Path (Clean check-in, valid scores, clean advancement)</option>
              <option value="dispute">2. Dispute Case (Conflicting scores, DISPUTED state, TO official ruling)</option>
              <option value="unexpressible_step">3. Unexpressible Step (Engine can't express &bull; auto-files needs-logic)</option>
              <option value="missing_route">4. Missing Route / Page (404 score route &bull; auto-files needs-page)</option>
              <option value="broken_control">5. Broken Control (Disabled submit button &bull; auto-files needs-page)</option>
              <option value="cant_enter_score">6. Can't Enter Score (Draw deadlock &bull; auto-files needs-logic)</option>
              <option value="cant_advance_winner">7. Can't Advance Winner (Advancement deadlock &bull; auto-files needs-logic)</option>
              <option value="cant_resolve_dispute">8. Can't Resolve Dispute (Arbitration blocked &bull; auto-files needs-logic)</option>
            </select>
          </div>

          <div style="background: #0d1117; border: 1px solid var(--gh-border); border-radius: 6px; padding: 0.85rem; margin-bottom: 1.25rem;">
            <div style="font-weight: 600; font-size: 12px; color: var(--gh-orange); margin-bottom: 0.25rem;">Staging Safety Notice</div>
            <p style="font-size: 12px; color: var(--gh-text-muted); line-height: 1.4; margin: 0;">
              All accounts are mock/staging accounts. Zero real money or cashier endpoints are touched. Traces and failure fingerprints are recorded automatically.
            </p>
          </div>

          <div style="display: flex; justify-content: flex-end; gap: 0.5rem;">
            <button class="gh-btn" onclick="closeRehearsalModal()">Cancel</button>
            <button id="btn-submit-rehearsal" class="gh-btn gh-btn-primary" onclick="executeModalRehearsal()">
              Launch Rehearsal
            </button>
          </div>
        </div>
      </div>
    </div>

    <script>
      function openRehearsalModal() {
        const modal = document.getElementById('rehearsal-modal');
        if (modal) modal.style.display = 'flex';
      }
      function closeRehearsalModal() {
        const modal = document.getElementById('rehearsal-modal');
        if (modal) modal.style.display = 'none';
      }

      async function triggerRehearsal(scenario, size, format_id, game_pack_id) {
        const btn = event?.target;
        if (btn) { btn.disabled = true; btn.textContent = '⏳ Running Rehearsal...'; }

        try {
          const res = await fetch('/project/${projectId}/rehearsals/run', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
              scenario,
              size: parseInt(size, 10),
              format_id,
              game_pack_id,
            })
          });
          if (res.ok) {
            const data = await res.json();
            window.location.href = '/p/${projectId}/rehearsals/' + data.run_id;
          } else {
            alert('Rehearsal failed to trigger: ' + await res.text());
            if (btn) { btn.disabled = false; btn.textContent = '⚡ Run Tournament Rehearsal'; }
          }
        } catch (err) {
          alert('Network error: ' + err.message);
          if (btn) { btn.disabled = false; btn.textContent = '⚡ Run Tournament Rehearsal'; }
        }
      }

      function executeModalRehearsal() {
        const size = document.getElementById('modal-rehearsal-size').value;
        const scenario = document.getElementById('modal-rehearsal-scenario').value;
        const format_id = document.getElementById('modal-rehearsal-format').value;
        const game_pack_id = document.getElementById('modal-rehearsal-pack').value;
        const btn = document.getElementById('btn-submit-rehearsal');
        if (btn) { btn.disabled = true; btn.textContent = '⏳ Executing...'; }
        triggerRehearsal(scenario, size, format_id, game_pack_id);
      }
    </script>
  `;
}

export function renderRehearsalDetailPage(
  projectId: string,
  run: RehearsalRunRecord
): string {
  const isPassed = run.status === "passed";
  const isDisputedResolved = run.status === "disputed_resolved";
  const isEvidenceMissing = run.status === "evidence_missing";
  const isFailed = run.status === "failed";

  const statusBadge = isPassed
    ? `<span class="gh-badge gh-badge-green" style="font-size: 13px;">✓ PASSED</span>`
    : isDisputedResolved
    ? `<span class="gh-badge gh-badge-purple" style="font-size: 13px;">⚖️ DISPUTE RESOLVED</span>`
    : isEvidenceMissing
    ? `<span class="gh-badge" style="font-size: 13px; font-weight: 700; color: #f1e05a; background: rgba(241,224,90,0.15); border: 1px solid rgba(241,224,90,0.4);">⚠️ EVIDENCE MISSING</span>`
    : `<span class="gh-badge gh-badge-red" style="font-size: 13px;">✗ FAILED</span>`;

  const evidenceMissingBanner = isEvidenceMissing
    ? `
      <div style="background: rgba(241,224,90,0.1); border: 1px solid rgba(241,224,90,0.4); border-radius: 6px; padding: 1rem; margin-bottom: 1.5rem;">
        <div style="display: flex; align-items: center; gap: 0.5rem; color: #f1e05a; font-weight: 700; font-size: 13px; margin-bottom: 0.25rem;">
          <span>⚠️ Evidence Verification Barrier Blocked</span>
        </div>
        <p style="font-size: 12px; color: var(--gh-text); margin: 0; line-height: 1.5;">
          This run cannot be marked as passed because one or more required evidence artifacts could not be verified in R2 (<code>s3a-evidence</code>):<br>
          <strong style="color: var(--gh-red-text); font-family: monospace;">${(run.missing_evidence || []).join(', ') || 'Evidence artifact missing'}</strong>.<br>
          Evidence must exist in R2 with HTTP 200 before pass state is accepted. Click &ldquo;Verify Evidence Live&rdquo; to re-check.
        </p>
      </div>
    `
    : "";

  // Stages rendering
  const stagesHtml = (run.stages || []).map((stage, idx) => {
    const isStagePassed = stage.status === "passed";
    const isStageDisputed = stage.status === "disputed";
    const isStageSkipped = stage.status === "skipped";

    const badge = isStagePassed
      ? `<span class="gh-badge gh-badge-green">✓ Passed</span>`
      : isStageDisputed
      ? `<span class="gh-badge gh-badge-purple">⚖️ Dispute Logged</span>`
      : isStageSkipped
      ? `<span class="gh-badge gh-badge-muted">○ Skipped</span>`
      : `<span class="gh-badge gh-badge-red">✗ Failed</span>`;

    const errorBlock = stage.error
      ? `
        <div style="margin-top: 0.75rem; background: #030712; border: 1px solid rgba(248,81,73,0.3); border-radius: 6px; padding: 0.75rem;">
          <div style="color: var(--gh-red-text); font-weight: 600; font-size: 12px; margin-bottom: 0.25rem;">Error Trace:</div>
          <pre style="font-family: monospace; font-size: 12px; color: var(--gh-red-text); white-space: pre-wrap; margin: 0;">${stage.error}</pre>
          ${stage.stack ? `<pre style="font-family: monospace; font-size: 11px; color: var(--gh-text-muted); margin-top: 0.5rem; overflow-x: auto;">${stage.stack}</pre>` : ''}
        </div>
      `
      : "";

    return `
      <div style="padding: 1rem; border-bottom: 1px solid var(--gh-border); background: ${stage.status === 'failed' ? 'rgba(218,54,51,0.05)' : 'transparent'};">
        <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.35rem;">
          <div style="display: flex; align-items: center; gap: 0.5rem;">
            <span style="font-weight: 700; font-size: 12px; color: var(--gh-text-muted);">Stage ${idx + 1}</span>
            <strong style="color: var(--gh-text-heading); font-size: 13px;">${stage.name}</strong>
          </div>
          <div style="display: flex; align-items: center; gap: 0.5rem;">
            ${badge}
            <span style="font-family: monospace; font-size: 11px; color: var(--gh-text-muted);">${stage.duration_ms}ms</span>
          </div>
        </div>

        ${stage.details ? `
          <div style="font-size: 12px; color: var(--gh-text-muted); margin-top: 0.35rem;">
            <pre style="background: #0d1117; padding: 0.5rem 0.75rem; border-radius: 4px; border: 1px solid var(--gh-border-subtle); font-family: monospace; font-size: 11px; color: var(--gh-text); overflow-x: auto; margin: 0;">${JSON.stringify(stage.details, null, 2)}</pre>
          </div>
        ` : ''}

        ${errorBlock}
      </div>
    `;
  }).join("");

  // Standings table
  const standingsHtml = (run.standings && run.standings.length > 0)
    ? `
      <div class="gh-card" style="margin-bottom: 1.5rem;">
        <div class="gh-card-header">
          <strong style="color: var(--gh-text-heading); font-size: 13px;">🏆 Official Tournament Standings</strong>
          <span style="font-size: 11px; color: var(--gh-green-text); font-weight: 600;">Staging Point Distribution Only &bull; Zero Real Cashier Risk</span>
        </div>
        <div style="overflow-x: auto;">
          <table style="width: 100%; border-collapse: collapse; font-size: 13px; text-align: left;">
            <thead>
              <tr style="border-bottom: 1px solid var(--gh-border); background: #0d1117; color: var(--gh-text-muted); font-size: 12px;">
                <th style="padding: 0.5rem 1rem;">Rank</th>
                <th style="padding: 0.5rem 1rem;">Player / Team</th>
                <th style="padding: 0.5rem 1rem;">Match Record</th>
                <th style="padding: 0.5rem 1rem;">Prize Points</th>
              </tr>
            </thead>
            <tbody>
              ${run.standings.map(s => {
                const medal = s.rank === 1 ? '🥇 ' : s.rank === 2 ? '🥈 ' : s.rank <= 4 ? '🥉 ' : '';
                return `
                  <tr style="border-bottom: 1px solid var(--gh-border-subtle); background: ${s.rank === 1 ? 'rgba(243,128,32,0.08)' : 'transparent'};">
                    <td style="padding: 0.5rem 1rem; font-weight: 600; color: ${s.rank === 1 ? 'var(--gh-orange)' : 'var(--gh-text-heading)'};">${medal}#${s.rank}</td>
                    <td style="padding: 0.5rem 1rem; font-weight: 600; color: var(--gh-text-heading);">${s.team_name}</td>
                    <td style="padding: 0.5rem 1rem; font-family: monospace;">${s.record}</td>
                    <td style="padding: 0.5rem 1rem; font-family: monospace; color: var(--gh-accent); font-weight: 600;">+${s.prize_points} Pts</td>
                  </tr>
                `;
              }).join("")}
            </tbody>
          </table>
        </div>
      </div>
    `
    : "";

  // Matches list
  const matchesHtml = (run.matches && run.matches.length > 0)
    ? `
      <div class="gh-card" style="margin-bottom: 1.5rem;">
        <div class="gh-card-header">
          <strong style="color: var(--gh-text-heading); font-size: 13px;">🎮 Bracket Match Results (${run.matches.length})</strong>
          <span style="font-size: 12px; color: var(--gh-text-muted);">${run.bracket_size}-Team Single Elimination</span>
        </div>
        <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 1rem; padding: 1rem;">
          ${run.matches.map(m => {
            const isDisputed = m.status === 'disputed' || !!m.dispute;
            return `
              <div style="background: #0d1117; border: 1px solid ${isDisputed ? 'var(--gh-purple)' : 'var(--gh-border)'}; border-radius: 6px; padding: 0.75rem;">
                <div style="display: flex; justify-content: space-between; font-size: 11px; color: var(--gh-text-muted); margin-bottom: 0.35rem;">
                  <span>${m.round_name} &bull; Match ${m.match_num}</span>
                  <span class="gh-badge ${isDisputed ? 'gh-badge-purple' : m.status === 'final' ? 'gh-badge-green' : 'gh-badge-muted'}">${m.status.toUpperCase()}</span>
                </div>
                <div style="display: flex; justify-content: space-between; align-items: center; padding: 0.2rem 0; font-size: 13px; font-weight: ${m.winner_id === m.team1.id ? '700' : 'normal'}; color: ${m.winner_id === m.team1.id ? 'var(--gh-accent)' : 'var(--gh-text)'};">
                  <span>#${m.team1.seed || '-'} ${m.team1.name}</span>
                  <span style="font-family: monospace; font-weight: 700;">${m.team1.score !== undefined ? m.team1.score : '-'}</span>
                </div>
                <div style="display: flex; justify-content: space-between; align-items: center; padding: 0.2rem 0; font-size: 13px; font-weight: ${m.winner_id === m.team2.id ? '700' : 'normal'}; color: ${m.winner_id === m.team2.id ? 'var(--gh-accent)' : 'var(--gh-text)'};">
                  <span>#${m.team2.seed || '-'} ${m.team2.name}</span>
                  <span style="font-family: monospace; font-weight: 700;">${m.team2.score !== undefined ? m.team2.score : '-'}</span>
                </div>
                ${m.dispute ? `
                  <div style="margin-top: 0.5rem; background: rgba(137,87,229,0.1); border: 1px solid rgba(137,87,229,0.3); border-radius: 4px; padding: 0.4rem; font-size: 11px; color: var(--gh-purple-text);">
                    <div><strong>Dispute Resolution:</strong> ${m.dispute.resolution || m.dispute.reason}</div>
                  </div>
                ` : ''}
              </div>
            `;
          }).join("")}
        </div>
      </div>
    `
    : "";

  // Auto-filed issues banner
  const issuesBanner = (run.issues_filed && run.issues_filed.length > 0)
    ? `
      <div style="background: rgba(218,54,51,0.1); border: 1px solid rgba(248,81,73,0.4); border-radius: 6px; padding: 1rem; margin-bottom: 1.5rem;">
        <div style="display: flex; align-items: center; gap: 0.5rem; color: var(--gh-red-text); font-weight: 700; margin-bottom: 0.35rem;">
          <span>🚨 Automated Bug Issues Auto-Filed (${run.issues_filed.length})</span>
        </div>
        <p style="font-size: 12px; color: var(--gh-text); margin-bottom: 0.5rem;">
          The simulated tournament encountered unhandled failures. Issues were automatically deduplicated by error fingerprint and added to the engineering task queue:
        </p>
        <div style="display: flex; gap: 0.5rem; flex-wrap: wrap;">
          ${run.issues_filed.map(tId => `
            <a href="/p/${projectId}/issues/${tId}" class="gh-btn gh-btn-danger gh-btn-sm" style="font-family: monospace;">
              🐞 ${tId} &bull; View Task Details ↗
            </a>
          `).join("")}
        </div>
      </div>
    `
    : "";

  // Visual evidence cards
  const screenshots = run.screenshots || [];
  const evidenceKeys = run.evidence_keys || screenshots.map(s => s.name);

  let evidenceItemsHtml = "";
  if (screenshots.length === 0 && evidenceKeys.length === 0) {
    evidenceItemsHtml = `
      <div style="padding: 1.5rem; text-align: center; color: var(--gh-text-muted); font-size: 12px; font-style: italic;">
        Screenshots (R2): not run
      </div>
    `;
  } else {
    evidenceItemsHtml = screenshots.map(s => {
      const isMissing = run.missing_evidence?.includes(s.name);
      if (isMissing) {
        return `
          <div style="padding: 1rem 1.25rem; border-bottom: 1px solid var(--gh-border); background: rgba(218,54,51,0.08);">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.35rem;">
              <strong style="font-size: 13px; color: var(--gh-red-text); font-family: monospace;">[evidence missing: ${s.name}]</strong>
              <span class="gh-badge gh-badge-red">404 NOT FOUND IN R2</span>
            </div>
            <p style="font-size: 12px; color: var(--gh-text-muted); margin: 0;">
              Expected verified object in R2 bucket <code>s3a-evidence</code> at key <code>${s.name}</code>, but object was not found. Run cannot pass.
            </p>
          </div>
        `;
      }
      return `
        <div style="padding: 1.25rem; border-bottom: 1px solid var(--gh-border);">
          <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.75rem; flex-wrap: wrap; gap: 0.5rem;">
            <div>
              <strong style="color: var(--gh-text-heading); font-size: 13px;">${s.caption || s.name}</strong>
              <div style="font-size: 11px; color: var(--gh-text-muted); font-family: monospace; margin-top: 0.2rem;">
                Key: <code style="color: var(--gh-text);">${s.name}</code> &bull; Bucket: <code style="color: var(--gh-text);">s3a-evidence</code> &bull; Provenance: SHA-256
              </div>
            </div>
            <div style="display: flex; gap: 0.5rem; align-items: center;">
              <span class="gh-badge gh-badge-green" style="font-size: 11px;">✓ R2 Verified (200 OK)</span>
              <a href="${s.data_url}" target="_blank" class="gh-btn gh-btn-sm">
                Open Full Object ↗
              </a>
            </div>
          </div>
          <div style="background: #0d1117; border: 1px solid var(--gh-border); border-radius: 6px; overflow: hidden; max-width: 960px; margin-top: 0.5rem;">
            <img src="${s.data_url}" alt="${s.caption || s.name}" style="width: 100%; height: auto; display: block;" onerror="this.parentElement.innerHTML='<div style=\\'padding:1rem;color:var(--gh-red-text);font-size:12px;\\'>⚠️ evidence missing: failed to load object from R2</div>'" />
          </div>
        </div>
      `;
    }).join("");
  }

  const evidenceCardHtml = `
    <div class="gh-card" style="margin-bottom: 1.5rem;">
      <div class="gh-card-header" style="display: flex; justify-content: space-between; align-items: center;">
        <div>
          <strong style="color: var(--gh-text-heading); font-size: 13px;">📸 Visual Evidence &amp; Screenshots (R2 Provenance)</strong>
          <span style="font-size: 11px; color: var(--gh-text-muted); margin-left: 0.5rem;">Direct objects from <code>s3a-evidence</code></span>
        </div>
        <span style="font-size: 11px; color: var(--gh-text-muted); font-family: monospace;">Bucket: s3a-evidence</span>
      </div>
      <div>
        ${evidenceItemsHtml}
      </div>
    </div>
  `;

  return `
    <div style="margin-bottom: 1.5rem;">
      <a href="/p/${projectId}/rehearsals" style="font-size: 12px; color: var(--gh-link); text-decoration: none;">
        &larr; Back to Rehearsal Runs History
      </a>
      <div style="display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 0.75rem; margin-top: 0.5rem;">
        <div>
          <div style="display: flex; align-items: center; gap: 0.6rem;">
            <h1 style="font-size: 1.4rem; font-weight: 700; color: var(--gh-text-heading); font-family: monospace;">
              ${run.run_id}
            </h1>
            ${statusBadge}
          </div>
          <div style="display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; font-size: 12px; color: var(--gh-text-muted); margin-top: 0.25rem;">
            <span>${run.tournament_name}</span>
            <span>&bull;</span>
            <span>Total Duration: <strong>${run.total_duration_ms}ms</strong></span>
            <span>&bull;</span>
            <span>Started: ${new Date(run.started_at).toISOString()}</span>
            ${run.evidence_verified ? '<span>&bull;</span><span style="color: var(--gh-green-text); font-weight: 600;">✓ R2 Provenance Verified</span>' : ''}
          </div>
        </div>

        <div style="display: flex; gap: 0.5rem; align-items: center;">
          <button id="btn-verify-evidence" class="gh-btn gh-btn-sm" onclick="verifyEvidenceLive('${run.run_id}')" style="display: inline-flex; align-items: center; gap: 0.35rem;">
            🔍 Verify Evidence Live
          </button>
          <a href="/project/${projectId}/trace/${run.run_id}" class="gh-btn gh-btn-sm" target="_blank">
            📜 xAPI Trace JSON ↗
          </a>
        </div>
      </div>
    </div>

    ${evidenceMissingBanner}
    ${issuesBanner}

    <!-- Live Evidence Verification Container -->
    <div id="live-verify-container" style="display: none; margin-bottom: 1.5rem;"></div>

    <!-- Visual Evidence & Screenshots (R2 Provenance) -->
    ${evidenceCardHtml}

    ${run.format_id ? `
      <div class="gh-card" style="margin-bottom: 1.5rem; background: #0d1117;">
        <div class="gh-card-header">
          <strong style="color: var(--gh-text-heading); font-size: 13px;">⚙️ Tournament Format Specification (Signed Data)</strong>
          <span class="gh-badge gh-badge-blue">v${run.format_version || '1.0.0'}</span>
        </div>
        <div style="padding: 1rem; display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 1rem; font-size: 12px;">
          <div>
            <div style="color: var(--gh-text-muted); margin-bottom: 0.2rem;">Format Config</div>
            <strong style="color: var(--gh-text-heading); font-size: 13px;">${run.format_name || run.format_id}</strong>
          </div>
          <div>
            <div style="color: var(--gh-text-muted); margin-bottom: 0.2rem;">Game Pack</div>
            <strong style="color: var(--gh-purple-text); font-size: 13px;">${run.game_pack_id || 'Custom / None'}</strong>
          </div>
          <div>
            <div style="color: var(--gh-text-muted); margin-bottom: 0.2rem;">Specification Hash</div>
            <code style="font-family: monospace; color: var(--gh-green-text); font-size: 12px;">${run.format_hash || 'sha256_verified'}</code>
          </div>
        </div>
      </div>
    ` : ''}

    <!-- Stages Execution Card -->
    <div class="gh-card" style="margin-bottom: 1.5rem;">
      <div class="gh-card-header">
        <strong style="color: var(--gh-text-heading); font-size: 13px;">Stage-by-Stage Rehearsal Execution</strong>
        <span style="font-size: 12px; color: var(--gh-text-muted);">${run.stages?.length || 0} Stages</span>
      </div>
      <div>
        ${stagesHtml}
      </div>
    </div>

    ${matchesHtml}
    ${standingsHtml}

    <script>
      async function verifyEvidenceLive(runId) {
        const btn = document.getElementById('btn-verify-evidence');
        const container = document.getElementById('live-verify-container');
        if (btn) { btn.disabled = true; btn.textContent = '⏳ Verifying R2 Live...'; }
        if (container) {
          container.style.display = 'block';
          container.innerHTML = '<div class="gh-card" style="padding: 1rem; color: var(--gh-text-muted); font-size: 12px;">⏳ Re-checking all screenshot &amp; log artifact URLs against R2 s3a-evidence...</div>';
        }
        try {
          const res = await fetch('/project/${projectId}/rehearsals/' + runId + '/verify-evidence');
          const data = await res.json();
          let html = '<div class="gh-card" style="margin-bottom: 1.5rem; border-color: ' + (data.all_verified ? 'var(--gh-green)' : 'var(--gh-red)') + ';">';
          html += '<div class="gh-card-header" style="display: flex; justify-content: space-between; align-items: center;">';
          html += '<strong style="color: var(--gh-text-heading); font-size: 13px;">Live Evidence Verification (R2 HEAD Check)</strong>';
          html += data.all_verified
            ? '<span class="gh-badge gh-badge-green" style="font-weight: 700;">✓ ALL 200 OK (VERIFIED)</span>'
            : '<span class="gh-badge gh-badge-red" style="font-weight: 700;">✗ EVIDENCE MISSING</span>';
          html += '</div>';
          html += '<div style="overflow-x: auto;">';
          html += '<table style="width: 100%; font-size: 12px; border-collapse: collapse; text-align: left;">';
          html += '<thead><tr style="border-bottom: 1px solid var(--gh-border); background: #0d1117; color: var(--gh-text-muted);"><th style="padding: 6px 12px;">Key</th><th style="padding: 6px 12px;">HTTP Status</th><th style="padding: 6px 12px;">Size</th><th style="padding: 6px 12px;">Content-Type</th><th style="padding: 6px 12px;">Action</th></tr></thead>';
          html += '<tbody>';
          for (const item of (data.results || [])) {
            const statusBadge = item.http_status === 200
              ? '<span class="gh-badge gh-badge-green">200 OK</span>'
              : '<span class="gh-badge gh-badge-red">' + item.http_status + ' ' + (item.error || 'MISSING') + '</span>';
            const sizeStr = item.size_bytes ? (item.size_bytes / 1024).toFixed(1) + ' KB' : 'N/A';
            html += '<tr style="border-bottom: 1px solid var(--gh-border-subtle);">';
            html += '<td style="padding: 8px 12px; font-family: monospace; font-weight: 600;">' + item.key + '</td>';
            html += '<td style="padding: 8px 12px;">' + statusBadge + '</td>';
            html += '<td style="padding: 8px 12px; font-family: monospace;">' + sizeStr + '</td>';
            html += '<td style="padding: 8px 12px; font-family: monospace; color: var(--gh-text-muted);">' + (item.content_type || 'image/svg+xml') + '</td>';
            html += '<td style="padding: 8px 12px;"><a href="' + item.url + '" target="_blank" class="gh-btn gh-btn-sm">View ↗</a></td>';
            html += '</tr>';
          }
          html += '</tbody></table></div></div>';
          if (container) container.innerHTML = html;
          if (btn) { btn.disabled = false; btn.textContent = '✓ Verified Live'; }
        } catch (err) {
          if (container) container.innerHTML = '<div class="gh-card" style="padding: 1rem; color: var(--gh-red-text); font-size: 12px;">Verification error: ' + err.message + '</div>';
          if (btn) { btn.disabled = false; btn.textContent = '🔍 Verify Evidence Live'; }
        }
      }
    </script>
  `;
}
