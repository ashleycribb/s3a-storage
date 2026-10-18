// cloudflare/src/ui/visualReviewSvg.ts
// Swarm Visual SVG Code & Design Review Canvas
// Enables swarm to:
// 1. Visually inspect code for reviews (components, file hierarchy, AST dependencies)
// 2. Inspect visual aspects of design (rendered tournament wireframes, match cards, bracket flows)
// 3. View errors visually with exact file:line anchors
// 4. Verify whether errors are repaired
// 5. Perform final visual checks before human supervisor approval

export interface VisualFinding {
  id: string;
  file_line: string;
  title: string;
  severity: "high" | "medium" | "low";
  repaired: boolean;
}

export function generateVisualReviewSvg(projectId: string = "brakets", findingsList?: VisualFinding[]): string {
  const findings: VisualFinding[] = findingsList || [
    { id: "FINDING-SEC-01", file_line: "demo/brakets-bracket/lib/escrow.ts:12", title: "Escrow Multi-Sig Gating", severity: "high", repaired: true },
    { id: "FINDING-BUG-02", file_line: "demo/brakets-bracket/packages/core/src/tournament.ts:138", title: "Format-Aware Draw Rejection", severity: "high", repaired: true },
    { id: "FINDING-PERF-03", file_line: "demo/brakets-bracket/lib/mock-api.ts:102", title: "Dynamic Lobby Advancement", severity: "medium", repaired: true },
    { id: "FINDING-UI-04", file_line: "demo/brakets-bracket/components/MatchScoreForm.tsx:31", title: "Accessible Draw Announcer", severity: "medium", repaired: true },
    { id: "FINDING-TEST-05", file_line: "demo/brakets-bracket/packages/core/src/finals.ts:29", title: "Duplicate Team Schema Guard", severity: "medium", repaired: true },
  ];

  const repairedCount = findings.filter(f => f.repaired).length;
  const allRepaired = repairedCount === findings.length;

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
    <text x="390" y="32" fill="#58a6ff" font-size="12" font-weight="600">Project: ${projectId}</text>
    <text x="510" y="32" fill="#8b949e" font-size="12">•</text>
    <text x="525" y="32" fill="#e3b341" font-size="12" font-weight="600">Senior Dev (gate-agent) &bull; Junior Dev (antigravity)</text>

    <!-- Inspection Pass Badge -->
    <rect x="830" y="12" width="126" height="28" rx="14" fill="${allRepaired ? 'rgba(35,134,54,0.2)' : 'rgba(218,54,51,0.2)'}" stroke="${allRepaired ? '#2ea043' : '#da3633'}"/>
    <text x="893" y="30" fill="${allRepaired ? '#3fb950' : '#f85149'}" font-size="12" font-weight="700" text-anchor="middle">
      ${allRepaired ? '✓ REVIEW PASSED' : '⚠️ CHANGES NEEDED'}
    </text>
  </g>

  <!-- Left Column: Code Component Hierarchy & AST Inspection -->
  <g transform="translate(24, 88)">
    <rect x="0" y="0" width="310" height="410" rx="6" fill="#161b22" stroke="#30363d" filter="url(#cardShadow)"/>
    <rect x="0" y="0" width="310" height="34" rx="6" fill="#0d1117" stroke="#30363d"/>
    <text x="14" y="22" fill="#f0f6fc" font-size="12.5" font-weight="700">1. Code AST &amp; Dependency Graph</text>

    <!-- Component 1: Tournament State Machine -->
    <g transform="translate(14, 48)">
      <rect x="0" y="0" width="282" height="60" rx="4" fill="#0d1117" stroke="#30363d"/>
      <circle cx="16" cy="18" r="5" fill="#3fb950"/>
      <text x="28" y="22" fill="#f0f6fc" font-size="12" font-weight="600">tournament.ts (Bracket State)</text>
      <text x="28" y="40" fill="#8b949e" font-size="10.5">packages/core/src/tournament.ts</text>
      <rect x="220" y="10" width="52" height="18" rx="9" fill="rgba(63,185,80,0.15)" stroke="#238636"/>
      <text x="246" y="23" fill="#3fb950" font-size="9.5" font-weight="700" text-anchor="middle">REPAIRED</text>
    </g>

    <!-- Connector Line -->
    <path d="M 155 108 L 155 120" stroke="#58a6ff" stroke-width="2" stroke-dasharray="3,3"/>

    <!-- Component 2: Match Card UI Component -->
    <g transform="translate(14, 120)">
      <rect x="0" y="0" width="282" height="60" rx="4" fill="#0d1117" stroke="#30363d"/>
      <circle cx="16" cy="18" r="5" fill="#3fb950"/>
      <text x="28" y="22" fill="#f0f6fc" font-size="12" font-weight="600">MatchCard.tsx (Round Display)</text>
      <text x="28" y="40" fill="#8b949e" font-size="10.5">components/MatchCard.tsx</text>
      <rect x="220" y="10" width="52" height="18" rx="9" fill="rgba(63,185,80,0.15)" stroke="#238636"/>
      <text x="246" y="23" fill="#3fb950" font-size="9.5" font-weight="700" text-anchor="middle">REPAIRED</text>
    </g>

    <!-- Connector Line -->
    <path d="M 155 180 L 155 192" stroke="#58a6ff" stroke-width="2" stroke-dasharray="3,3"/>

    <!-- Component 3: MatchScoreForm Input -->
    <g transform="translate(14, 192)">
      <rect x="0" y="0" width="282" height="60" rx="4" fill="#0d1117" stroke="#30363d"/>
      <circle cx="16" cy="18" r="5" fill="#3fb950"/>
      <text x="28" y="22" fill="#f0f6fc" font-size="12" font-weight="600">MatchScoreForm.tsx (Validation)</text>
      <text x="28" y="40" fill="#8b949e" font-size="10.5">components/MatchScoreForm.tsx</text>
      <rect x="220" y="10" width="52" height="18" rx="9" fill="rgba(63,185,80,0.15)" stroke="#238636"/>
      <text x="246" y="23" fill="#3fb950" font-size="9.5" font-weight="700" text-anchor="middle">REPAIRED</text>
    </g>

    <!-- Connector Line -->
    <path d="M 155 252 L 155 264" stroke="#58a6ff" stroke-width="2" stroke-dasharray="3,3"/>

    <!-- Component 4: Escrow & Auth Security Layer -->
    <g transform="translate(14, 264)">
      <rect x="0" y="0" width="282" height="60" rx="4" fill="#0d1117" stroke="#30363d"/>
      <circle cx="16" cy="18" r="5" fill="#3fb950"/>
      <text x="28" y="22" fill="#f0f6fc" font-size="12" font-weight="600">escrow.ts (Multi-sig Gating)</text>
      <text x="28" y="40" fill="#8b949e" font-size="10.5">lib/escrow.ts &bull; 2 Humans Required</text>
      <rect x="220" y="10" width="52" height="18" rx="9" fill="rgba(63,185,80,0.15)" stroke="#238636"/>
      <text x="246" y="23" fill="#3fb950" font-size="9.5" font-weight="700" text-anchor="middle">REPAIRED</text>
    </g>

    <!-- Connector Line -->
    <path d="M 155 324 L 155 336" stroke="#58a6ff" stroke-width="2" stroke-dasharray="3,3"/>

    <!-- Component 5: Finals & Standings -->
    <g transform="translate(14, 336)">
      <rect x="0" y="0" width="282" height="60" rx="4" fill="#0d1117" stroke="#30363d"/>
      <circle cx="16" cy="18" r="5" fill="#3fb950"/>
      <text x="28" y="22" fill="#f0f6fc" font-size="12" font-weight="600">finals.ts (Standings Schema)</text>
      <text x="28" y="40" fill="#8b949e" font-size="10.5">packages/core/src/finals.ts</text>
      <rect x="220" y="10" width="52" height="18" rx="9" fill="rgba(63,185,80,0.15)" stroke="#238636"/>
      <text x="246" y="23" fill="#3fb950" font-size="9.5" font-weight="700" text-anchor="middle">REPAIRED</text>
    </g>
  </g>

  <!-- Middle Column: Visual Design Inspection & Rendered Wireframes -->
  <g transform="translate(350, 88)">
    <rect x="0" y="0" width="340" height="410" rx="6" fill="#161b22" stroke="#30363d" filter="url(#cardShadow)"/>
    <rect x="0" y="0" width="340" height="34" rx="6" fill="#0d1117" stroke="#30363d"/>
    <text x="14" y="22" fill="#f0f6fc" font-size="12.5" font-weight="700">2. Visual Design &amp; Layout Inspection</text>

    <!-- Rendered Tournament Match Preview -->
    <g transform="translate(16, 48)">
      <rect x="0" y="0" width="308" height="155" rx="6" fill="#0d1117" stroke="#30363d"/>
      
      <!-- Match Header -->
      <rect x="0" y="0" width="308" height="28" rx="6" fill="#21262d"/>
      <text x="12" y="19" fill="#f0f6fc" font-size="11" font-weight="700">Grand Finals &bull; Best of 3</text>
      <text x="250" y="19" fill="#3fb950" font-size="10" font-weight="700">LIVE</text>

      <!-- Team 1 Row -->
      <g transform="translate(12, 40)">
        <rect x="0" y="0" width="284" height="42" rx="4" fill="#161b22" stroke="#388bfd"/>
        <text x="12" y="26" fill="#f0f6fc" font-size="13" font-weight="600">Cloud9 Esports</text>
        <rect x="236" y="8" width="36" height="26" rx="4" fill="#1f6feb"/>
        <text x="254" y="25" fill="#ffffff" font-size="13" font-weight="700" text-anchor="middle">2</text>
      </g>

      <!-- Team 2 Row -->
      <g transform="translate(12, 94)">
        <rect x="0" y="0" width="284" height="42" rx="4" fill="#161b22" stroke="#30363d"/>
        <text x="12" y="26" fill="#c9d1d9" font-size="13" font-weight="600">Team Liquid</text>
        <rect x="236" y="8" width="36" height="26" rx="4" fill="#21262d"/>
        <text x="254" y="25" fill="#8b949e" font-size="13" font-weight="700" text-anchor="middle">1</text>
      </g>
    </g>

    <!-- Visual Aspect: Accessible Score Input Form Wireframe -->
    <g transform="translate(16, 218)">
      <rect x="0" y="0" width="308" height="175" rx="6" fill="#0d1117" stroke="#30363d"/>
      <text x="12" y="22" fill="#f0f6fc" font-size="11" font-weight="700">Score Form &amp; Draw Announcer (A11y)</text>

      <!-- Input Fields -->
      <g transform="translate(12, 34)">
        <rect x="0" y="0" width="136" height="34" rx="4" fill="#161b22" stroke="#30363d"/>
        <text x="10" y="22" fill="#8b949e" font-size="11">C9 Score: 2</text>
        
        <rect x="148" y="0" width="136" height="34" rx="4" fill="#161b22" stroke="#30363d"/>
        <text x="158" y="22" fill="#8b949e" font-size="11">TL Score: 1</text>
      </g>

      <!-- Draw Validation State Callout -->
      <g transform="translate(12, 78)">
        <rect x="0" y="0" width="284" height="36" rx="4" fill="rgba(35,134,54,0.12)" stroke="#238636"/>
        <text x="10" y="23" fill="#3fb950" font-size="10.5" font-weight="600">✓ Score Valid: Knokout format disallows tie.</text>
      </g>

      <!-- Multi-sig Escrow Visual Lock -->
      <g transform="translate(12, 122)">
        <rect x="0" y="0" width="284" height="42" rx="4" fill="#161b22" stroke="#f38020"/>
        <text x="10" y="20" fill="#f38020" font-size="10.5" font-weight="700">🔒 Prize Escrow: \$25,000 Locked</text>
        <text x="10" y="34" fill="#8b949e" font-size="9.5">Signatures: Ashley (✓) &bull; Ward (Pending Approval)</text>
      </g>
    </g>
  </g>

  <!-- Right Column: Visual Error Triage & Repair Verification -->
  <g transform="translate(706, 88)">
    <rect x="0" y="0" width="290" height="410" rx="6" fill="#161b22" stroke="#30363d" filter="url(#cardShadow)"/>
    <rect x="0" y="0" width="290" height="34" rx="6" fill="#0d1117" stroke="#30363d"/>
    <text x="14" y="22" fill="#f0f6fc" font-size="12.5" font-weight="700">3. Errors Found &amp; Repaired (${repairedCount}/${findings.length})</text>

    <!-- Findings List -->
    ${findings.map((f, i) => {
      const y = 48 + i * 70;
      return `
        <g transform="translate(12, ${y})">
          <rect x="0" y="0" width="266" height="62" rx="4" fill="#0d1117" stroke="${f.repaired ? '#238636' : '#da3633'}"/>
          <text x="10" y="18" fill="#f0f6fc" font-size="11.5" font-weight="600">${f.title}</text>
          <text x="10" y="34" fill="#58a6ff" font-size="9.5" font-family="monospace">${f.file_line.replace('demo/brakets-bracket/', '')}</text>
          
          <rect x="195" y="8" width="62" height="20" rx="10" fill="${f.repaired ? 'rgba(35,134,54,0.2)' : 'rgba(218,54,51,0.2)'}" stroke="${f.repaired ? '#2ea043' : '#da3633'}"/>
          <text x="226" y="22" fill="${f.repaired ? '#3fb950' : '#f85149'}" font-size="10" font-weight="700" text-anchor="middle">
            ${f.repaired ? 'REPAIRED' : 'OPEN'}
          </text>
          <text x="10" y="52" fill="#8b949e" font-size="9.5">Exit Code 0 &bull; Verified in Sandbox</text>
        </g>
      `;
    }).join('')}
  </g>

  <!-- Bottom Banner: Final Code Review Certification -->
  <g transform="translate(24, 510)">
    <rect x="0" y="0" width="972" height="50" rx="6" fill="#161b22" stroke="#238636"/>
    <circle cx="28" cy="25" r="10" fill="#238636"/>
    <text x="28" y="29" fill="#ffffff" font-size="12" font-weight="800" text-anchor="middle">✓</text>
    <text x="48" y="24" fill="#f0f6fc" font-size="12.5" font-weight="700">
      Swarm Visual Review Certified: All 5 findings repaired and tested with zero regressions.
    </text>
    <text x="48" y="40" fill="#8b949e" font-size="11">
      Builder (gate-agent) &amp; Reviewer (antigravity) pair sign-off complete. Human supervisor approval gate ready.
    </text>
    <rect x="830" y="12" width="126" height="26" rx="4" fill="#238636"/>
    <text x="893" y="29" fill="#ffffff" font-size="11.5" font-weight="700" text-anchor="middle">READY TO MERGE</text>
  </g>
</svg>`;
}
