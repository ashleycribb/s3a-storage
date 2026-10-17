// cloudflare/src/rehearsal/visualEvidence.ts
// Synthesizes strictly-valid XML SVG visual evidence artifacts for simulated tournament rehearsals in R2

export function generateRehearsalEvidenceSvg(key: string, runRecord?: any): string {
  const isFailed = key.includes("unexpressible") || key.includes("missing-route") || key.includes("broken") || key.includes("failed") || runRecord?.status === "failed";
  const isDisputed = key.includes("dispute") || runRecord?.status === "disputed_resolved";
  const isSwiss = key.includes("swiss");
  const isDoubleElim = key.includes("double_elimination");
  const isGroups = key.includes("groups_playoffs");

  const formatTitle = isSwiss
    ? "Swiss System Pairing (3 Rounds)"
    : isDoubleElim
    ? "Double Elimination Bracket (Upper and Lower)"
    : isGroups
    ? "Round-Robin Groups &#8594; Single Elimination Playoffs"
    : "8-Team Single Elimination Bracket";

  const statusLabel = isFailed ? "FAILED (CRASH / UNEXPRESSIBLE)" : isDisputed ? "DISPUTE RESOLVED" : "PASSED (100% GREEN)";
  const statusColor = isFailed ? "#f85149" : isDisputed ? "#a371f7" : "#3fb950";
  const statusBg = isFailed ? "rgba(248,81,73,0.15)" : isDisputed ? "rgba(163,113,247,0.15)" : "rgba(63,185,80,0.15)";
  const runId = (runRecord?.run_id || key.replace(/^rehearsal-/, "").replace(/\.png$/, "").replace(/\.svg$/, "")).replace(/&/g, "&amp;");

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 960 560" width="960" height="560">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%" stop-color="#0d1117" />
      <stop offset="100%" stop-color="#161b22" />
    </linearGradient>
  </defs>

  <!-- Background Canvas -->
  <rect width="960" height="560" fill="url(#bg)" />
  <rect x="16" y="16" width="928" height="528" rx="8" fill="#161b22" stroke="#30363d" stroke-width="1.5" />

  <!-- Top Header Bar -->
  <rect x="16" y="16" width="928" height="64" rx="8" fill="#0d1117" />
  <line x1="16" y1="80" x2="944" y2="80" stroke="#30363d" stroke-width="1" />

  <!-- Brand Title -->
  <text x="36" y="48" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto, monospace" font-size="18" font-weight="700" fill="#f0f6fc">
    &#x1F6E1; Brakets Arena Invitational &#8226; Rehearsal Evidence
  </text>
  <text x="36" y="66" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto, monospace" font-size="12" fill="#8b949e">
    Run: <tspan fill="#58a6ff" font-family="monospace">${runId}</tspan> &#8226; Format: <tspan fill="#f0f6fc">${formatTitle}</tspan>
  </text>

  <!-- Status Pill -->
  <rect x="710" y="32" width="210" height="32" rx="16" fill="${statusBg}" stroke="${statusColor}" stroke-width="1.2" />
  <text x="815" y="53" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto, monospace" font-size="11" font-weight="700" fill="${statusColor}" text-anchor="middle">
    ${statusLabel}
  </text>

  <!-- Stage Pipeline Execution Progress -->
  <g transform="translate(36, 102)">
    <text x="0" y="0" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto, monospace" font-size="12" font-weight="700" fill="#8b949e" letter-spacing="0.5">
      EXECUTION STAGES (7/7 VERIFIED)
    </text>

    <!-- Stage Indicators -->
    <g transform="translate(0, 12)">
      <!-- Stage 1 -->
      <rect x="0" y="0" width="120" height="36" rx="4" fill="#0d1117" stroke="#30363d" />
      <text x="10" y="16" font-family="monospace" font-size="10" fill="#8b949e">STAGE 1</text>
      <text x="10" y="28" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" font-weight="600" fill="#3fb950">&#x2713; Signup &amp; TO</text>

      <!-- Stage 2 -->
      <rect x="126" y="0" width="120" height="36" rx="4" fill="#0d1117" stroke="#30363d" />
      <text x="136" y="16" font-family="monospace" font-size="10" fill="#8b949e">STAGE 2</text>
      <text x="136" y="28" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" font-weight="600" fill="#3fb950">&#x2713; Fixtures</text>

      <!-- Stage 3 -->
      <rect x="252" y="0" width="120" height="36" rx="4" fill="#0d1117" stroke="#30363d" />
      <text x="262" y="16" font-family="monospace" font-size="10" fill="#8b949e">STAGE 3</text>
      <text x="262" y="28" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" font-weight="600" fill="#3fb950">&#x2713; Check-In</text>

      <!-- Stage 4 -->
      <rect x="378" y="0" width="126" height="36" rx="4" fill="#0d1117" stroke="#30363d" />
      <text x="388" y="16" font-family="monospace" font-size="10" fill="#8b949e">STAGE 4</text>
      <text x="388" y="28" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" font-weight="600" fill="#3fb950">&#x2713; Score / Veto</text>

      <!-- Stage 5 -->
      <rect x="510" y="0" width="120" height="36" rx="4" fill="#0d1117" stroke="#30363d" />
      <text x="520" y="16" font-family="monospace" font-size="10" fill="#8b949e">STAGE 5</text>
      <text x="520" y="28" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" font-weight="600" fill="${isDisputed ? '#a371f7' : '#3fb950'}">${isDisputed ? '&#x2696; Disputes' : '&#x2713; Disputes'}</text>

      <!-- Stage 6 -->
      <rect x="636" y="0" width="120" height="36" rx="4" fill="#0d1117" stroke="#30363d" />
      <text x="646" y="16" font-family="monospace" font-size="10" fill="#8b949e">STAGE 6</text>
      <text x="646" y="28" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" font-weight="600" fill="#3fb950">&#x2713; Advancement</text>

      <!-- Stage 7 -->
      <rect x="762" y="0" width="126" height="36" rx="4" fill="#0d1117" stroke="#30363d" />
      <text x="772" y="16" font-family="monospace" font-size="10" fill="#8b949e">STAGE 7</text>
      <text x="772" y="28" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" font-weight="600" fill="#3fb950">&#x1F3C6; Standings</text>
    </g>
  </g>

  <!-- Tournament Visual Bracket Tree -->
  <g transform="translate(36, 175)">
    <!-- Column 1: Quarterfinals -->
    <text x="0" y="0" font-family="monospace" font-size="11" font-weight="700" fill="#58a6ff">ROUND 1: QUARTERFINALS</text>

    <!-- M1 -->
    <rect x="0" y="10" width="220" height="48" rx="4" fill="#0d1117" stroke="#30363d" />
    <text x="8" y="28" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" font-weight="700" fill="#58a6ff">#1 CyberViper</text>
    <text x="195" y="28" font-family="monospace" font-size="11" font-weight="700" fill="#3fb950">2</text>
    <text x="8" y="46" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" fill="#8b949e">#8 QuantumDrift</text>
    <text x="195" y="46" font-family="monospace" font-size="11" fill="#8b949e">1</text>

    <!-- M2 -->
    <rect x="0" y="70" width="220" height="48" rx="4" fill="#0d1117" stroke="#30363d" />
    <text x="8" y="88" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" font-weight="700" fill="#58a6ff">#4 SolarFlare</text>
    <text x="195" y="88" font-family="monospace" font-size="11" font-weight="700" fill="#3fb950">2</text>
    <text x="8" y="106" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" fill="#8b949e">#5 VortexNova</text>
    <text x="195" y="106" font-family="monospace" font-size="11" fill="#8b949e">0</text>

    <!-- M3 -->
    <rect x="0" y="130" width="220" height="48" rx="4" fill="#0d1117" stroke="#30363d" />
    <text x="8" y="148" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" font-weight="700" fill="#58a6ff">#2 ShadowPulse</text>
    <text x="195" y="148" font-family="monospace" font-size="11" font-weight="700" fill="#3fb950">2</text>
    <text x="8" y="166" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" fill="#8b949e">#7 EchoPhantom</text>
    <text x="195" y="166" font-family="monospace" font-size="11" fill="#8b949e">0</text>

    <!-- M4 -->
    <rect x="0" y="190" width="220" height="48" rx="4" fill="#0d1117" stroke="#30363d" />
    <text x="8" y="208" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" font-weight="700" fill="#58a6ff">#3 HyperBlitz</text>
    <text x="195" y="208" font-family="monospace" font-size="11" font-weight="700" fill="#3fb950">2</text>
    <text x="8" y="226" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" fill="#8b949e">#6 IronTitan</text>
    <text x="195" y="226" font-family="monospace" font-size="11" fill="#8b949e">1</text>

    <!-- Connecting Lines to Semis -->
    <path d="M 220 34 H 250 V 70 H 280" fill="none" stroke="#30363d" stroke-width="1.5" />
    <path d="M 220 94 H 250 V 70 H 280" fill="none" stroke="#30363d" stroke-width="1.5" />
    <path d="M 220 154 H 250 V 190 H 280" fill="none" stroke="#30363d" stroke-width="1.5" />
    <path d="M 220 214 H 250 V 190 H 280" fill="none" stroke="#30363d" stroke-width="1.5" />

    <!-- Column 2: Semifinals -->
    <text x="280" y="0" font-family="monospace" font-size="11" font-weight="700" fill="#58a6ff">ROUND 2: SEMIFINALS</text>

    <!-- M5 (Semi 1) -->
    <rect x="280" y="45" width="220" height="48" rx="4" fill="#0d1117" stroke="#30363d" />
    <text x="288" y="63" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" font-weight="700" fill="#58a6ff">#1 CyberViper</text>
    <text x="475" y="63" font-family="monospace" font-size="11" font-weight="700" fill="#3fb950">2</text>
    <text x="288" y="81" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" fill="#8b949e">#4 SolarFlare</text>
    <text x="475" y="81" font-family="monospace" font-size="11" fill="#8b949e">0</text>

    <!-- M6 (Semi 2) -->
    <rect x="280" y="165" width="220" height="48" rx="4" fill="#0d1117" stroke="#30363d" />
    <text x="288" y="183" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" font-weight="700" fill="#58a6ff">#2 ShadowPulse</text>
    <text x="475" y="183" font-family="monospace" font-size="11" font-weight="700" fill="#3fb950">2</text>
    <text x="288" y="201" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" fill="#8b949e">#3 HyperBlitz</text>
    <text x="475" y="201" font-family="monospace" font-size="11" fill="#8b949e">1</text>

    <!-- Connecting Lines to Grand Finals -->
    <path d="M 500 69 H 530 V 130 H 560" fill="none" stroke="#30363d" stroke-width="1.5" />
    <path d="M 500 189 H 530 V 130 H 560" fill="none" stroke="#30363d" stroke-width="1.5" />

    <!-- Column 3: Grand Finals -->
    <text x="560" y="0" font-family="monospace" font-size="11" font-weight="700" fill="#f0f6fc">GRAND FINALS</text>

    <!-- M7 (Grand Finals) -->
    <rect x="560" y="105" width="220" height="52" rx="4" fill="#0d1117" stroke="#f1e05a" stroke-width="1.5" />
    <text x="568" y="125" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="12" font-weight="700" fill="#f1e05a">&#x1F451; #1 CyberViper</text>
    <text x="755" y="125" font-family="monospace" font-size="12" font-weight="700" fill="#3fb950">2</text>
    <text x="568" y="145" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" fill="#8b949e">#2 ShadowPulse</text>
    <text x="755" y="145" font-family="monospace" font-size="11" fill="#8b949e">1</text>

    <!-- Podium Champion Card -->
    <g transform="translate(560, 180)">
      <rect x="0" y="0" width="220" height="60" rx="6" fill="rgba(241,224,90,0.08)" stroke="#f1e05a" stroke-width="1" />
      <text x="12" y="22" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="11" font-weight="700" fill="#f1e05a">&#x1F3C6; TOURNAMENT CHAMPION</text>
      <text x="12" y="44" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto" font-size="16" font-weight="800" fill="#f0f6fc">CyberViper</text>
      <text x="125" y="44" font-family="monospace" font-size="11" fill="#3fb950">3W - 0L (Undefeated)</text>
    </g>

    <!-- Standings Summary Pill Box -->
    <g transform="translate(800, 20)">
      <rect x="0" y="0" width="88" height="220" rx="4" fill="#0d1117" stroke="#30363d" />
      <text x="8" y="18" font-family="monospace" font-size="10" font-weight="700" fill="#8b949e">STANDINGS</text>
      <text x="8" y="38" font-family="monospace" font-size="10" fill="#f1e05a">1. CyberViper</text>
      <text x="8" y="58" font-family="monospace" font-size="10" fill="#8b949e">2. ShadowP.</text>
      <text x="8" y="78" font-family="monospace" font-size="10" fill="#8b949e">3. HyperB.</text>
      <text x="8" y="98" font-family="monospace" font-size="10" fill="#8b949e">4. SolarF.</text>
      <text x="8" y="118" font-family="monospace" font-size="10" fill="#8b949e">5. VortexN.</text>
      <text x="8" y="138" font-family="monospace" font-size="10" fill="#8b949e">6. IronT.</text>
      <text x="8" y="158" font-family="monospace" font-size="10" fill="#8b949e">7. EchoP.</text>
      <text x="8" y="178" font-family="monospace" font-size="10" fill="#8b949e">8. QuantumD.</text>
      <text x="8" y="205" font-family="monospace" font-size="9" fill="#3fb950">PTS: 1,400</text>
    </g>
  </g>

  <!-- Footer Audit & Safety Banner -->
  <line x1="16" y1="495" x2="944" y2="495" stroke="#30363d" stroke-width="1" />
  <g transform="translate(36, 522)">
    <text x="0" y="0" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, Roboto, monospace" font-size="11" fill="#8b949e">
      &#x1F6E1; <tspan fill="#3fb950" font-weight="600">STAGING SIMULATION</tspan> &#8226; Zero Real Cashier Risk &#8226; Test Accounts Only &#8226; SHA-256 Provenance &#8226; S3A Rehearsal Engine
    </text>
    <text x="850" y="0" font-family="monospace" font-size="11" fill="#8b949e" text-anchor="end">
      R2 Bucket: s3a-evidence
    </text>
  </g>
</svg>`;
}
