// cloudflare/src/ui/visualReviewSvg.ts
// Swarm Visual SVG Code & Design Review Canvas
// Enables swarm to:
// 1. Visually inspect code for reviews (components, file hierarchy, AST dependencies)
// 2. Inspect visual aspects of design (rendered tournament wireframes, match cards, bracket flows)
// 3. View errors visually with exact file:line anchors
// 4. Verify whether errors are repaired against real stored CR/run records
// 5. Perform final visual checks before human supervisor approval

export interface VisualFindingRun {
  id?: string;
  taskId: string;
  crId?: string;
  file_line: string;
  component: string;
  agent: string;
  goal: string;
  title: string;
  risk_level?: "high" | "low" | string;
  exit_code?: number;
  duration_ms?: number;
  commit_sha?: string;
  log_url?: string;
  passed?: boolean;
  status?: string; // "failed_verification" | "pending_review" | "approved" | "merged" | "not_run"
  first_human_approval?: {
    reviewer_pubkey?: string;
    supervisor_name?: string;
    signature?: string;
  };
  second_human_approval?: {
    reviewer_pubkey?: string;
    supervisor_name?: string;
    signature?: string;
  };
  reviewer_pubkey?: string;
}

export interface LegacyVisualFinding {
  id: string;
  file_line: string;
  title: string;
  severity: "high" | "medium" | "low";
  repaired?: boolean;
}

function escapeXml(unsafe: string): string {
  return (unsafe || "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}

export const DEFAULT_FINDINGS_SPEC: VisualFindingRun[] = [
  {
    taskId: "TASK-FINDING-TOURNAMENT-138",
    component: "tournament.ts (Bracket State)",
    agent: "helper-tournament-core",
    goal: "Format-aware draw rejection",
    file_line: "demo/brakets-bracket/packages/core/src/tournament.ts:138",
    title: "Format-Aware Draw Rejection",
    risk_level: "high",
  },
  {
    taskId: "TASK-FINDING-MATCHCARD-49",
    component: "MatchCard.tsx (Round Display)",
    agent: "helper-a11y-ui",
    goal: "Contrast, A11y & correct round labels",
    file_line: "demo/brakets-bracket/components/MatchCard.tsx:49",
    title: "MatchCard Round Hierarchy",
    risk_level: "low",
  },
  {
    taskId: "TASK-FINDING-SCOREFORM-31",
    component: "MatchScoreForm.tsx (Validation)",
    agent: "helper-a11y-ui",
    goal: "Live aria draw validation announcer",
    file_line: "demo/brakets-bracket/components/MatchScoreForm.tsx:31",
    title: "Accessible Draw Announcer",
    risk_level: "low",
  },
  {
    taskId: "TASK-FINDING-ESCROW-12",
    component: "escrow.ts (Multi-sig Gating)",
    agent: "helper-sec-guard",
    goal: "Ed25519 2-human multisig required",
    file_line: "demo/brakets-bracket/lib/escrow.ts:12",
    title: "Escrow Multi-Sig Gating",
    risk_level: "high",
  },
  {
    taskId: "TASK-FINDING-FINALS-29",
    component: "finals.ts (Standings Schema)",
    agent: "helper-qa-auditor",
    goal: "Duplicate team ID guard & test pass",
    file_line: "demo/brakets-bracket/packages/core/src/finals.ts:29",
    title: "Duplicate Team Schema Guard",
    risk_level: "low",
  },
];

export function generateVisualReviewSvg(
  projectId: string = "brakets",
  findingsList?: (VisualFindingRun | LegacyVisualFinding)[]
): string {
  // Normalize findings list from input or canonical spec
  const findings: VisualFindingRun[] = (findingsList && findingsList.length > 0)
    ? findingsList.map((item, idx) => {
        const spec = DEFAULT_FINDINGS_SPEC[idx] || DEFAULT_FINDINGS_SPEC[0];
        const isRun = "taskId" in item;
        if (isRun) {
          return item as VisualFindingRun;
        }
        // Convert legacy finding if passed
        const legacy = item as LegacyVisualFinding;
        return {
          ...spec,
          title: legacy.title || spec.title,
          file_line: legacy.file_line || spec.file_line,
          passed: legacy.repaired === true,
          exit_code: legacy.repaired ? 0 : 254,
          status: legacy.repaired ? "approved" : "failed_verification",
        };
      })
    : DEFAULT_FINDINGS_SPEC.map(spec => ({
        ...spec,
        // When no real run is passed, default to not run (never fabricated exit 0 or repaired)
        status: "not_run",
        passed: false,
        exit_code: undefined,
      }));

  // Evaluation of real verification status
  const passingFindings = findings.filter(f => f.passed === true && f.exit_code === 0 && f.status !== "failed_verification");
  const failedFindings = findings.filter(f => f.exit_code !== undefined && (!f.passed || f.exit_code !== 0 || f.status === "failed_verification"));
  const passedCount = passingFindings.length;
  const failedCount = failedFindings.length;
  const anyFailed = failedCount > 0;

  // Ready to merge invariant:
  // "Ready to merge" appears only when the CR passes the Needs judgment barrier AND has the required human signatures. Agents never set it.
  const allReadyToMerge = findings.length > 0 && findings.every(f => {
    if (!f.passed || f.exit_code !== 0 || f.status === "failed_verification") return false;
    if (!f.commit_sha || f.commit_sha === "not run") return false;
    if (f.risk_level === "high") {
      return Boolean(f.first_human_approval?.signature && f.second_human_approval?.signature);
    }
    return Boolean(f.reviewer_pubkey || f.status === "approved" || f.first_human_approval?.signature);
  });

  // Escrow signature summary for middle wireframe column
  const escrowFinding = findings.find(f => f.taskId === "TASK-FINDING-ESCROW-12");
  let escrowSignaturesText = "Signatures: Pending Supervisor Signatures (Ashley + Ward required)";
  if (escrowFinding?.first_human_approval && escrowFinding?.second_human_approval) {
    escrowSignaturesText = `Signatures: ${escapeXml(escrowFinding.first_human_approval.supervisor_name || "Ashley")} (✓) • ${escapeXml(escrowFinding.second_human_approval.supervisor_name || "Ward")} (✓)`;
  } else if (escrowFinding?.first_human_approval) {
    escrowSignaturesText = `Signatures: ${escapeXml(escrowFinding.first_human_approval.supervisor_name || "Supervisor")} (✓) • 2nd Supervisor Required (Pending)`;
  }

  // Top header status badge
  let topBadgeBg = "rgba(218,54,51,0.2)";
  let topBadgeStroke = "#da3633";
  let topBadgeTextColor = "#f85149";
  let topBadgeText = "⚠ VERIFICATION FAILED";
  let topBadgeWidth = 160;

  if (allReadyToMerge) {
    topBadgeBg = "rgba(35,134,54,0.2)";
    topBadgeStroke = "#2ea043";
    topBadgeTextColor = "#3fb950";
    topBadgeText = "✓ READY TO MERGE";
    topBadgeWidth = 140;
  } else if (!anyFailed && passedCount === findings.length) {
    topBadgeBg = "rgba(227,179,65,0.2)";
    topBadgeStroke = "#d29922";
    topBadgeTextColor = "#e3b341";
    topBadgeText = "⏳ AWAITING SIGNATURES";
    topBadgeWidth = 166;
  } else if (!anyFailed && passedCount === 0) {
    topBadgeBg = "rgba(139,148,158,0.2)";
    topBadgeStroke = "#8b949e";
    topBadgeTextColor = "#8b949e";
    topBadgeText = "⏳ RUNS PENDING";
    topBadgeWidth = 130;
  }

  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1020 580" width="100%" height="100%" style="background: #0d1117; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;">
  <defs>
    <linearGradient id="bgGrad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#161b22"/>
      <stop offset="100%" stop-color="#0d1117"/>
    </linearGradient>
    <linearGradient id="accentGrad" x1="0%" y1="0%" x2="100%" y2="0%">
      <stop offset="0%" stop-color="#f38020"/>
      <stop offset="100%" stop-color="#e3b341"/>
    </linearGradient>
    <filter id="cardShadow" x="-5%" y="-5%" width="110%" height="110%">
      <feDropShadow dx="0" dy="4" stdDeviation="6" flood-color="#010409" flood-opacity="0.6"/>
    </filter>
  </defs>

  <!-- Canvas Background -->
  <rect width="1020" height="580" fill="url(#bgGrad)"/>
  <rect x="1" y="1" width="1018" height="578" fill="none" stroke="#30363d" stroke-width="1.5" rx="8"/>

  <!-- Top Navigation & Header -->
  <g transform="translate(24, 20)">
    <rect x="0" y="0" width="972" height="52" rx="6" fill="#161b22" stroke="#30363d"/>
    <text x="16" y="32" fill="#f38020" font-size="16" font-weight="800" letter-spacing="-0.5">swarm</text>
    <text x="76" y="32" fill="#8b949e" font-size="12">▶</text>
    <text x="96" y="32" fill="#f0f6fc" font-size="14" font-weight="700">Visual Code &amp; Design Review Canvas</text>
    <text x="375" y="32" fill="#8b949e" font-size="12">•</text>
    <text x="390" y="32" fill="#58a6ff" font-size="12" font-weight="600">Project: ${escapeXml(projectId)}</text>
    <text x="510" y="32" fill="#e3b341" font-size="11" font-weight="600">Senior Dev (gate-agent) &bull; Junior Dev (antigravity)</text>

    <!-- Inspection Pass / Verification Badge -->
    <rect x="${972 - topBadgeWidth - 14}" y="12" width="${topBadgeWidth}" height="28" rx="14" fill="${topBadgeBg}" stroke="${topBadgeStroke}"/>
    <text x="${972 - (topBadgeWidth / 2) - 14}" y="30" fill="${topBadgeTextColor}" font-size="11" font-weight="700" text-anchor="middle">
      ${topBadgeText}
    </text>
  </g>

  <!-- Left Column: Coordinated Helper Agents & Tasks -->
  <g transform="translate(24, 88)">
    <rect x="0" y="0" width="310" height="410" rx="6" fill="#161b22" stroke="#30363d" filter="url(#cardShadow)"/>
    <rect x="0" y="0" width="310" height="34" rx="6" fill="#0d1117" stroke="#30363d"/>
    <text x="14" y="22" fill="#f0f6fc" font-size="12.5" font-weight="700">1. Coordinated Helper Agents &amp; Tasks</text>

    ${findings.map((f, i) => {
      const y = 46 + i * 72;
      const isPass = f.passed === true && f.exit_code === 0 && f.status !== "failed_verification";
      const isFail = f.exit_code !== undefined && !isPass;

      let badgeBg = "rgba(139,148,158,0.15)";
      let badgeStroke = "#8b949e";
      let badgeTextColor = "#8b949e";
      let badgeText = "NOT RUN";
      let statusDotColor = "#8b949e";

      if (isPass) {
        badgeBg = "rgba(63,185,80,0.15)";
        badgeStroke = "#238636";
        badgeTextColor = "#3fb950";
        badgeText = "PASSED";
        statusDotColor = "#3fb950";
      } else if (isFail) {
        badgeBg = "rgba(218,54,51,0.15)";
        badgeStroke = "#da3633";
        badgeTextColor = "#f85149";
        badgeText = `FAILED (${f.exit_code})`;
        statusDotColor = "#f85149";
      }

      return `
        <g transform="translate(14, ${y})">
          <rect x="0" y="0" width="282" height="64" rx="4" fill="#0d1117" stroke="#30363d"/>
          <circle cx="16" cy="18" r="5" fill="${statusDotColor}"/>
          <text x="28" y="22" fill="#f0f6fc" font-size="11.5" font-weight="600">${escapeXml(f.component)}</text>
          <text x="28" y="38" fill="#58a6ff" font-size="9.5">Agent: ${escapeXml(f.agent)}</text>
          <text x="28" y="52" fill="#8b949e" font-size="9">Goal: ${escapeXml(f.goal)}</text>
          <rect x="194" y="10" width="80" height="18" rx="9" fill="${badgeBg}" stroke="${badgeStroke}"/>
          <text x="234" y="22" fill="${badgeTextColor}" font-size="8.5" font-weight="700" text-anchor="middle">${badgeText}</text>
        </g>
        ${i < findings.length - 1 ? `<path d="M 155 ${y + 64} L 155 ${y + 72}" stroke="#58a6ff" stroke-width="1.5" stroke-dasharray="2,2"/>` : ''}
      `;
    }).join('')}
  </g>

  <!-- Middle Column: Visual Design Inspection & Rendered Wireframes (EXAMPLE DATA) -->
  <g transform="translate(350, 88)">
    <rect x="0" y="0" width="340" height="410" rx="6" fill="#161b22" stroke="#30363d" filter="url(#cardShadow)"/>
    <rect x="0" y="0" width="340" height="34" rx="6" fill="#0d1117" stroke="#30363d"/>
    <text x="14" y="22" fill="#f0f6fc" font-size="12" font-weight="700">2. Visual Design &amp; Wireframes [EXAMPLE DATA]</text>

    <!-- Rendered Tournament Match Preview -->
    <g transform="translate(16, 46)">
      <rect x="0" y="0" width="308" height="155" rx="6" fill="#0d1117" stroke="#30363d"/>
      
      <!-- Match Header -->
      <rect x="0" y="0" width="308" height="28" rx="6" fill="#21262d"/>
      <text x="12" y="19" fill="#f0f6fc" font-size="11" font-weight="700">Grand Finals &bull; Best of 3 [EXAMPLE DATA]</text>
      <rect x="252" y="5" width="46" height="18" rx="4" fill="#30363d"/>
      <text x="275" y="17" fill="#8b949e" font-size="8.5" font-weight="700" text-anchor="middle">MOCK</text>

      <!-- Team 1 Row -->
      <g transform="translate(12, 38)">
        <rect x="0" y="0" width="284" height="42" rx="4" fill="#161b22" stroke="#388bfd"/>
        <text x="12" y="26" fill="#f0f6fc" font-size="12" font-weight="600">Cloud9 Esports [EXAMPLE DATA]</text>
        <rect x="236" y="8" width="36" height="26" rx="4" fill="#1f6feb"/>
        <text x="254" y="25" fill="#ffffff" font-size="13" font-weight="700" text-anchor="middle">2</text>
      </g>

      <!-- Team 2 Row -->
      <g transform="translate(12, 92)">
        <rect x="0" y="0" width="284" height="42" rx="4" fill="#161b22" stroke="#30363d"/>
        <text x="12" y="26" fill="#c9d1d9" font-size="12" font-weight="600">Team Liquid [EXAMPLE DATA]</text>
        <rect x="236" y="8" width="36" height="26" rx="4" fill="#21262d"/>
        <text x="254" y="25" fill="#8b949e" font-size="13" font-weight="700" text-anchor="middle">1</text>
      </g>
    </g>

    <!-- Visual Aspect: Accessible Score Input Form Wireframe -->
    <g transform="translate(16, 214)">
      <rect x="0" y="0" width="308" height="180" rx="6" fill="#0d1117" stroke="#30363d"/>
      <text x="12" y="20" fill="#f0f6fc" font-size="11" font-weight="700">Score Form &amp; Draw Announcer [EXAMPLE DATA]</text>

      <!-- Input Fields -->
      <g transform="translate(12, 30)">
        <rect x="0" y="0" width="136" height="34" rx="4" fill="#161b22" stroke="#30363d"/>
        <text x="10" y="22" fill="#8b949e" font-size="10.5">C9 Score: 2</text>
        
        <rect x="148" y="0" width="136" height="34" rx="4" fill="#161b22" stroke="#30363d"/>
        <text x="158" y="22" fill="#8b949e" font-size="10.5">TL Score: 1</text>
      </g>

      <!-- Draw Validation State Callout -->
      <g transform="translate(12, 74)">
        <rect x="0" y="0" width="284" height="36" rx="4" fill="rgba(35,134,54,0.12)" stroke="#238636"/>
        <text x="10" y="23" fill="#3fb950" font-size="9.5" font-weight="600">&check; Score Valid: Knockout format disallows tie. [EXAMPLE DATA]</text>
      </g>

      <!-- Multi-sig Escrow Visual Lock -->
      <g transform="translate(12, 118)">
        <rect x="0" y="0" width="284" height="48" rx="4" fill="#161b22" stroke="#f38020"/>
        <text x="10" y="18" fill="#f38020" font-size="10" font-weight="700">🔒 Prize Escrow: \$25,000 [EXAMPLE DATA]</text>
        <text x="10" y="34" fill="#8b949e" font-size="8.5">${escrowSignaturesText}</text>
      </g>
    </g>
  </g>

  <!-- Right Column: Visual Error Triage & Real Verification Status -->
  <g transform="translate(706, 88)">
    <rect x="0" y="0" width="290" height="410" rx="6" fill="#161b22" stroke="#30363d" filter="url(#cardShadow)"/>
    <rect x="0" y="0" width="290" height="34" rx="6" fill="#0d1117" stroke="#30363d"/>
    <text x="14" y="22" fill="#f0f6fc" font-size="12" font-weight="700">3. Verification Status (${passedCount}/${findings.length} passing)</text>

    <!-- Findings List from Real Stored Records -->
    ${findings.map((f, i) => {
      const y = 46 + i * 72;
      const isPass = f.passed === true && f.exit_code === 0 && f.status !== "failed_verification";
      const isFail = f.exit_code !== undefined && !isPass;

      let cardStroke = "#30363d";
      let badgeBg = "rgba(139,148,158,0.15)";
      let badgeStroke = "#8b949e";
      let badgeTextColor = "#8b949e";
      let badgeText = "NOT RUN";

      if (isPass) {
        cardStroke = "#238636";
        badgeBg = "rgba(63,185,80,0.15)";
        badgeStroke = "#238636";
        badgeTextColor = "#3fb950";
        badgeText = "PASSED";
      } else if (isFail) {
        cardStroke = "#da3633";
        badgeBg = "rgba(218,54,51,0.15)";
        badgeStroke = "#da3633";
        badgeTextColor = "#f85149";
        badgeText = `FAILED (${f.exit_code})`;
      }

      // Real execution details
      let detailsLine1 = "not run &bull; no sandbox execution";
      let detailsLine2 = "";

      if (isPass) {
        detailsLine1 = `exit 0${f.duration_ms ? ` &bull; ${f.duration_ms}ms` : ''}${f.commit_sha ? ` &bull; ${escapeXml(f.commit_sha.slice(0, 7))}` : ''}`;
        if (f.log_url) {
          const logName = f.log_url.split('/').pop() || 'r2-build.log';
          detailsLine2 = `R2: ${escapeXml(logName.slice(0, 32))}`;
        }
      } else if (isFail) {
        detailsLine1 = `failed (${f.exit_code})${f.duration_ms ? ` &bull; ${f.duration_ms}ms` : ''}${f.commit_sha ? ` &bull; ${escapeXml(f.commit_sha.slice(0, 7))}` : ''}`;
        if (f.log_url) {
          const logName = f.log_url.split('/').pop() || 'r2-build.log';
          detailsLine2 = `R2: ${escapeXml(logName.slice(0, 32))}`;
        } else {
          detailsLine2 = "workspace_missing (ENOENT)";
        }
      }

      return `
        <g transform="translate(12, ${y})">
          <rect x="0" y="0" width="266" height="64" rx="4" fill="#0d1117" stroke="${cardStroke}"/>
          <text x="10" y="17" fill="#f0f6fc" font-size="11" font-weight="600">${escapeXml(f.title)}</text>
          <text x="10" y="31" fill="#58a6ff" font-size="8.5" font-family="monospace">${escapeXml(f.file_line.replace('demo/brakets-bracket/', ''))}</text>
          
          <rect x="180" y="8" width="76" height="18" rx="9" fill="${badgeBg}" stroke="${badgeStroke}"/>
          <text x="218" y="21" fill="${badgeTextColor}" font-size="8.5" font-weight="700" text-anchor="middle">
            ${badgeText}
          </text>
          
          <text x="10" y="45" fill="${isPass ? '#3fb950' : (isFail ? '#f85149' : '#8b949e')}" font-size="8.5">
            ${detailsLine1}
          </text>
          ${detailsLine2 ? `<text x="10" y="56" fill="#8b949e" font-size="7.5" font-family="monospace">${detailsLine2}</text>` : ''}
        </g>
      `;
    }).join('')}
  </g>

  <!-- Bottom Banner: Final Code Review Certification & Merge Guard -->
  <g transform="translate(24, 510)">
    ${allReadyToMerge ? `
      <rect x="0" y="0" width="972" height="50" rx="6" fill="#161b22" stroke="#238636"/>
      <circle cx="28" cy="25" r="10" fill="#238636"/>
      <text x="28" y="29" fill="#ffffff" font-size="12" font-weight="800" text-anchor="middle">&check;</text>
      <text x="48" y="24" fill="#f0f6fc" font-size="12.5" font-weight="700">
        Swarm Visual Review Certified: All ${findings.length} findings verified with exit code 0 and human supervisor signatures.
      </text>
      <text x="48" y="40" fill="#8b949e" font-size="11">
        Human supervisor approval verified (Ashley + Ward signed). Ready for merge into main.
      </text>
      <rect x="830" y="12" width="126" height="26" rx="4" fill="#238636"/>
      <text x="893" y="29" fill="#ffffff" font-size="11.5" font-weight="700" text-anchor="middle">READY TO MERGE</text>
    ` : (anyFailed ? `
      <rect x="0" y="0" width="972" height="50" rx="6" fill="#161b22" stroke="#da3633"/>
      <circle cx="28" cy="25" r="10" fill="#da3633"/>
      <text x="28" y="29" fill="#ffffff" font-size="12" font-weight="800" text-anchor="middle">!</text>
      <text x="48" y="24" fill="#f85149" font-size="12" font-weight="700">
        Verification Incomplete: ${failedCount}/${findings.length} tasks failed verification (non-zero exit code).
      </text>
      <text x="48" y="40" fill="#8b949e" font-size="11">
        Requires passing sandbox build (exit 0), fresh commit SHA, and human supervisor signatures (Ashley + Ward).
      </text>
      <rect x="800" y="12" width="156" height="26" rx="4" fill="#da3633"/>
      <text x="878" y="29" fill="#ffffff" font-size="11" font-weight="700" text-anchor="middle">FAILED VERIFICATION</text>
    ` : `
      <rect x="0" y="0" width="972" height="50" rx="6" fill="#161b22" stroke="#d29922"/>
      <circle cx="28" cy="25" r="10" fill="#d29922"/>
      <text x="28" y="29" fill="#ffffff" font-size="12" font-weight="800" text-anchor="middle">⏳</text>
      <text x="48" y="24" fill="#e3b341" font-size="12" font-weight="700">
        Verification Pending: Sandbox runs passed (${passedCount}/${findings.length}) but awaiting supervisor signatures.
      </text>
      <text x="48" y="40" fill="#8b949e" font-size="11">
        Agents cannot self-merge: Human supervisor signatures (Ashley + Ward) are strictly required.
      </text>
      <rect x="780" y="12" width="176" height="26" rx="4" fill="#9e6a03"/>
      <text x="868" y="29" fill="#ffffff" font-size="11" font-weight="700" text-anchor="middle">AWAITING SUPERVISOR</text>
    `)}
  </g>
</svg>`;
}
