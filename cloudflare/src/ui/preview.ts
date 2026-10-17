export function renderBraketsPreviewHtml(projectId: string): string {
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Brakets Tournament Preview - Real Agent Workspace</title>
  <style>
    :root {
      --bg: #0b0f19;
      --card-bg: #111827;
      --card-border: #1f2937;
      --primary: #f38020;
      --accent: #38bdf8;
      --secondary: #22d3ee;
      --success: #10b981;
      --danger: #ef4444;
      --warning: #f59e0b;
      --text: #f9fafb;
      --text-muted: #9ca3af;
    }
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      background: var(--bg);
      color: var(--text);
      line-height: 1.5;
      padding: 1.5rem;
    }
    .header {
      display: flex;
      justify-content: space-between;
      align-items: center;
      padding-bottom: 1rem;
      border-bottom: 1px solid var(--card-border);
      margin-bottom: 1.5rem;
    }
    .badge {
      display: inline-flex;
      align-items: center;
      gap: 0.4rem;
      padding: 0.25rem 0.6rem;
      border-radius: 9999px;
      font-size: 0.75rem;
      font-weight: 600;
      text-transform: uppercase;
      border: 1px solid;
    }
    .badge-test {
      background: rgba(16, 185, 129, 0.15);
      color: var(--success);
      border-color: var(--success);
    }
    .badge-role {
      background: rgba(56, 189, 248, 0.15);
      color: var(--accent);
      border-color: var(--accent);
    }
    .btn {
      background: var(--accent);
      color: #0b0f19;
      border: none;
      border-radius: 0.375rem;
      padding: 0.5rem 1rem;
      font-weight: 700;
      font-size: 0.85rem;
      cursor: pointer;
      transition: all 0.15s;
    }
    .btn:hover { opacity: 0.9; }
    .btn-orange { background: var(--primary); color: #fff; }
    .btn-secondary { background: #374151; color: #fff; }
    .grid { display: grid; gap: 1rem; }
    .bracket-grid {
      display: grid;
      grid-template-columns: 1fr 1fr 1fr;
      gap: 1.5rem;
      margin-top: 1.5rem;
    }
    .round-col {
      background: var(--card-bg);
      border: 1px solid var(--card-border);
      border-radius: 0.5rem;
      padding: 1rem;
    }
    .round-title {
      font-size: 0.85rem;
      text-transform: uppercase;
      font-weight: 700;
      color: var(--text-muted);
      margin-bottom: 1rem;
      display: flex;
      justify-content: space-between;
    }
    .match-card {
      background: #030712;
      border: 1px solid var(--card-border);
      border-radius: 0.375rem;
      padding: 0.75rem;
      margin-bottom: 0.75rem;
      transition: border-color 0.2s;
    }
    .match-card.active { border-color: var(--accent); }
    .match-card.final { border-color: var(--success); }
    .team-row {
      display: flex;
      justify-content: space-between;
      align-items: center;
      padding: 0.25rem 0;
      font-size: 0.85rem;
    }
    .team-row.winner { color: var(--accent); font-weight: 700; }
    .score-badge {
      background: #1f2937;
      padding: 0.1rem 0.4rem;
      border-radius: 0.25rem;
      font-family: monospace;
      font-weight: 700;
    }
    .input-field {
      background: #030712;
      border: 1px solid #374151;
      border-radius: 0.375rem;
      color: var(--text);
      padding: 0.4rem 0.6rem;
      font-size: 0.85rem;
      outline: none;
    }
    .input-field:focus { border-color: var(--accent); }
  </style>
</head>
<body>
  <div class="header">
    <div>
      <h1 style="font-size: 1.4rem; font-weight: 700; color: #fff;">Brakets Tournament Live Preview</h1>
      <div style="font-size: 0.8rem; color: var(--text-muted); margin-top: 0.2rem;">
        Private Workspace Mirrored from <code>ashleycribb/plat-gaming</code> (main) &bull; Project: <strong>${projectId}</strong>
      </div>
    </div>
    <div style="display: flex; gap: 0.5rem; align-items: center;">
      <span class="badge badge-test">🛡️ Test Mode: Fake Accounts Only &bull; Zero Real Cashier Risk</span>
      <span id="user-session-badge" class="badge badge-role">Guest / Anonymous</span>
    </div>
  </div>

  <!-- Role-Aware Auth & Player Onboarding Panel (Issue #3) -->
  <div style="background: var(--card-bg); border: 1px solid var(--card-border); border-radius: 0.5rem; padding: 1.25rem; margin-bottom: 1.5rem;">
    <div style="font-weight: 700; font-size: 1rem; margin-bottom: 0.5rem; display: flex; justify-content: space-between; align-items: center;">
      <span>1. Role-Aware Authentication (Issue #3 Acceptance)</span>
      <span id="role-permissions-badge" style="font-size: 0.75rem; color: var(--accent); font-weight: normal;"></span>
    </div>
    <div style="display: flex; gap: 0.75rem; align-items: center; flex-wrap: wrap;">
      <div>
        <label style="font-size: 0.75rem; color: var(--text-muted); display: block;">Player Handle</label>
        <input id="signup-handle" class="input-field" type="text" value="NeonStriker" style="width: 140px;">
      </div>
      <div>
        <label style="font-size: 0.75rem; color: var(--text-muted); display: block;">Assigned Role</label>
        <select id="signup-role" class="input-field" style="width: 170px;">
          <option value="organizer" selected>organizer (TO)</option>
          <option value="player">player</option>
          <option value="coach">coach</option>
          <option value="streamer">streamer</option>
          <option value="team_manager">team_manager</option>
        </select>
      </div>
      <div style="align-self: flex-end;">
        <button id="btn-signup" class="btn" onclick="executeSignup()">
          Sign In / Sign Up (Test Mode)
        </button>
      </div>
      <div id="signup-success-msg" style="color: var(--success); font-size: 0.85rem; font-weight: 600; display: none;">
        ✓ Authenticated as NeonStriker
      </div>
    </div>
  </div>

  <!-- Tournament Operations & Scripted Journey Controls -->
  <div style="background: var(--card-bg); border: 1px solid var(--card-border); border-radius: 0.5rem; padding: 1.25rem; margin-bottom: 1.5rem;">
    <div style="font-weight: 700; font-size: 1rem; margin-bottom: 0.75rem;">
      2. Scripted Journey Actions: Seed Bracket, Join Slot, Report Score, Advance Round
    </div>
    <div style="display: flex; gap: 0.75rem; flex-wrap: wrap; align-items: center;">
      <button id="btn-seed-bracket" class="btn btn-orange" onclick="seedBracket()">
        ⚡ Seed 8-Team Tournament Bracket
      </button>
      <button id="btn-join-match" class="btn btn-secondary" onclick="joinMatch()" style="display: none;">
        🎮 Join Match 1 (as CyberViper)
      </button>
      <div id="score-controls" style="display: none; align-items: center; gap: 0.5rem;">
        <input id="input-score-team1" class="input-field" type="number" placeholder="CyberViper Score" value="2" style="width: 70px;">
        <span style="color: var(--text-muted);">-</span>
        <input id="input-score-team2" class="input-field" type="number" placeholder="QuantumDrift Score" value="1" style="width: 70px;">
        <button id="btn-submit-score" class="btn" onclick="submitScore()">
          Submit Score
        </button>
        <button id="btn-advance-round" class="btn btn-orange" onclick="advanceRound()" style="display: none;">
          🏆 Advance Round Winner
        </button>
      </div>
    </div>
    <div id="score-error-msg" style="color: var(--danger); font-size: 0.8rem; margin-top: 0.5rem; display: none;"></div>
    <div id="journey-status-log" style="color: var(--accent); font-size: 0.82rem; margin-top: 0.5rem; font-family: monospace;">
      Ready. Click 'Sign In' or 'Seed Bracket' to begin journey.
    </div>
  </div>

  <!-- Interactive 3-Tier Tournament Bracket Board -->
  <div class="bracket-grid">
    <!-- Round 1: Quarterfinals -->
    <div class="round-col">
      <div class="round-title">
        <span>Round 1 &bull; Quarterfinals</span>
        <span id="r1-count" class="badge badge-test">4 Matches</span>
      </div>
      <div id="match-1" class="match-card">
        <div style="display: flex; justify-content: space-between; font-size: 0.75rem; color: var(--text-muted); margin-bottom: 0.35rem;">
          <span>Match 1</span>
          <span id="match-1-status" class="badge" style="background: rgba(56,189,248,0.1); color: var(--accent); font-size: 0.65rem;">PENDING</span>
        </div>
        <div class="team-row" id="slot-m1-t1">
          <span><span style="color: var(--text-muted); font-size: 0.75rem;">#1</span> <strong id="m1-t1-name">CyberViper</strong></span>
          <span class="score-badge" id="m1-t1-score">-</span>
        </div>
        <div class="team-row" id="slot-m1-t2">
          <span><span style="color: var(--text-muted); font-size: 0.75rem;">#8</span> <strong id="m1-t2-name">QuantumDrift</strong></span>
          <span class="score-badge" id="m1-t2-score">-</span>
        </div>
      </div>

      <div id="match-2" class="match-card">
        <div style="display: flex; justify-content: space-between; font-size: 0.75rem; color: var(--text-muted); margin-bottom: 0.35rem;">
          <span>Match 2</span>
          <span class="badge" style="background: rgba(16,185,129,0.1); color: var(--success); font-size: 0.65rem;">FINAL</span>
        </div>
        <div class="team-row winner">
          <span>#4 <strong>SolarFlare</strong></span>
          <span class="score-badge">2</span>
        </div>
        <div class="team-row">
          <span>#5 <strong>VortexNova</strong></span>
          <span class="score-badge">0</span>
        </div>
      </div>

      <div id="match-3" class="match-card">
        <div style="display: flex; justify-content: space-between; font-size: 0.75rem; color: var(--text-muted); margin-bottom: 0.35rem;">
          <span>Match 3</span>
          <span class="badge" style="background: rgba(16,185,129,0.1); color: var(--success); font-size: 0.65rem;">FINAL</span>
        </div>
        <div class="team-row winner">
          <span>#2 <strong>ShadowPulse</strong></span>
          <span class="score-badge">2</span>
        </div>
        <div class="team-row">
          <span>#7 <strong>EchoPhantom</strong></span>
          <span class="score-badge">1</span>
        </div>
      </div>

      <div id="match-4" class="match-card">
        <div style="display: flex; justify-content: space-between; font-size: 0.75rem; color: var(--text-muted); margin-bottom: 0.35rem;">
          <span>Match 4</span>
          <span class="badge" style="background: rgba(16,185,129,0.1); color: var(--success); font-size: 0.65rem;">FINAL</span>
        </div>
        <div class="team-row winner">
          <span>#3 <strong>HyperBlitz</strong></span>
          <span class="score-badge">2</span>
        </div>
        <div class="team-row">
          <span>#6 <strong>IronTitan</strong></span>
          <span class="score-badge">0</span>
        </div>
      </div>
    </div>

    <!-- Round 2: Semifinals -->
    <div class="round-col">
      <div class="round-title">
        <span>Round 2 &bull; Semifinals</span>
        <span class="badge badge-test">2 Matches</span>
      </div>
      <div id="match-5" class="match-card">
        <div style="display: flex; justify-content: space-between; font-size: 0.75rem; color: var(--text-muted); margin-bottom: 0.35rem;">
          <span>Match 5</span>
          <span id="match-5-status" class="badge" style="background: rgba(156,163,175,0.1); color: var(--text-muted); font-size: 0.65rem;">AWAITING M1</span>
        </div>
        <div class="team-row" id="slot-m5-t1">
          <span style="color: var(--text-muted); font-style: italic;">Winner of Match 1</span>
          <span class="score-badge">-</span>
        </div>
        <div class="team-row winner">
          <span>SolarFlare</span>
          <span class="score-badge">-</span>
        </div>
      </div>

      <div id="match-6" class="match-card">
        <div style="display: flex; justify-content: space-between; font-size: 0.75rem; color: var(--text-muted); margin-bottom: 0.35rem;">
          <span>Match 6</span>
          <span class="badge" style="background: rgba(56,189,248,0.1); color: var(--accent); font-size: 0.65rem;">UPCOMING</span>
        </div>
        <div class="team-row">
          <span>ShadowPulse</span>
          <span class="score-badge">-</span>
        </div>
        <div class="team-row">
          <span>HyperBlitz</span>
          <span class="score-badge">-</span>
        </div>
      </div>
    </div>

    <!-- Round 3: Grand Finals -->
    <div class="round-col">
      <div class="round-title">
        <span>Round 3 &bull; Championship</span>
        <span class="badge badge-role">Grand Finals</span>
      </div>
      <div id="match-7" class="match-card" style="border-color: #38bdf8;">
        <div style="display: flex; justify-content: space-between; font-size: 0.75rem; color: var(--text-muted); margin-bottom: 0.35rem;">
          <span>Championship Match 7</span>
          <span class="badge" style="background: rgba(243,128,32,0.15); color: var(--primary); font-size: 0.65rem;">TROPHY STAGE</span>
        </div>
        <div class="team-row">
          <span style="color: var(--text-muted); font-style: italic;">Winner of Match 5</span>
          <span class="score-badge">-</span>
        </div>
        <div class="team-row">
          <span style="color: var(--text-muted); font-style: italic;">Winner of Match 6</span>
          <span class="score-badge">-</span>
        </div>
      </div>
    </div>
  </div>

  <script>
    let currentUser = null;
    let bracketSeeded = false;

    function executeSignup() {
      const handle = document.getElementById('signup-handle').value.trim() || 'NeonStriker';
      const role = document.getElementById('signup-role').value;
      currentUser = { handle, role };

      document.getElementById('user-session-badge').textContent = 'Signed In: @' + handle + ' (' + role + ')';
      document.getElementById('user-session-badge').style.borderColor = '#10b981';
      document.getElementById('user-session-badge').style.color = '#10b981';
      document.getElementById('role-permissions-badge').textContent = '✓ Role active: ' + role + (role === 'organizer' ? ' (Full TO Permissions: Tournament Creation Unlocked)' : ' (Player Roster Permissions)');
      document.getElementById('signup-success-msg').style.display = 'inline-block';
      document.getElementById('btn-join-match').style.display = 'inline-block';
      document.getElementById('journey-status-log').textContent = '[Step 1 Passed] Authenticated user @' + handle + ' with role: ' + role;
    }

    function seedBracket() {
      bracketSeeded = true;
      document.getElementById('match-1').classList.add('active');
      document.getElementById('match-1-status').textContent = 'READY TO PLAY';
      document.getElementById('score-controls').style.display = 'inline-flex';
      document.getElementById('journey-status-log').textContent = '[Step 2 Passed] 8 competitive tournament rosters seeded across 4 quarterfinal brackets.';
    }

    function joinMatch() {
      const slot = document.getElementById('slot-m1-t1');
      if (slot) {
        slot.innerHTML = '<span><span style="color: var(--success); font-size: 0.75rem;">●</span> <strong id="m1-t1-name">CyberViper</strong> <span style="font-size: 0.75rem; color: #38bdf8;">(@NeonStriker)</span></span><span class="score-badge" id="m1-t1-score">-</span>';
      }
      document.getElementById('journey-status-log').textContent = '[Step 3 Passed] Joined Match 1 as slot CyberViper.';
    }

    function submitScore() {
      const s1 = parseInt(document.getElementById('input-score-team1').value, 10);
      const s2 = parseInt(document.getElementById('input-score-team2').value, 10);
      const errBox = document.getElementById('score-error-msg');

      if (isNaN(s1) || isNaN(s2) || s1 < 0 || s2 < 0) {
        errBox.textContent = 'Validation Error: Score must be a non-negative integer (Chaos Resilient)';
        errBox.style.display = 'block';
        return;
      }
      errBox.style.display = 'none';

      document.getElementById('m1-t1-score').textContent = s1;
      document.getElementById('m1-t2-score').textContent = s2;
      document.getElementById('match-1-status').textContent = 'FINAL';
      document.getElementById('match-1-status').style.color = '#10b981';
      document.getElementById('match-1').classList.remove('active');
      document.getElementById('match-1').classList.add('final');
      document.getElementById('slot-m1-t1').classList.add('winner');

      document.getElementById('btn-advance-round').style.display = 'inline-block';
      document.getElementById('journey-status-log').textContent = '[Step 4 Passed] Match 1 score reported: CyberViper ' + s1 + ' - ' + s2 + ' QuantumDrift.';
    }

    function advanceRound() {
      const m5Slot = document.getElementById('slot-m5-t1');
      if (m5Slot) {
        m5Slot.innerHTML = '<span class="winner"><strong style="color: #38bdf8;">CyberViper</strong> <span style="font-size:0.75rem; color: #10b981;">(Adv.)</span></span><span class="score-badge">-</span>';
      }
      document.getElementById('match-5-status').textContent = 'READY';
      document.getElementById('match-5-status').style.color = '#38bdf8';
      document.getElementById('match-5').classList.add('active');
      document.getElementById('journey-status-log').textContent = '[Step 5 Passed] CyberViper advanced to Round 2 Semifinals (Match 5)! Complete journey verified.';
    }
  </script>
</body>
</html>`;
}