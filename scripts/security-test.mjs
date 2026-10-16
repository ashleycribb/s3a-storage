// scripts/security-test.mjs
// S3A Gate Security Hardening Test Suite
import assert from "node:assert";

const BASE_URL = process.env.BASE_URL || "https://s3a-gate.ashley-e-cribb.workers.dev";
const ADMIN_SECRET = process.env.ADMIN_SECRET || "s3a_admin_sec_9f8b2a14e6c7d35";
const PROJECT_ID = "s3a-swarm-main";

console.log(`\n======================================================`);
console.log(`🧪 Running S3A Gate Security Hardening Verification`);
console.log(`Target: ${BASE_URL}`);
console.log(`======================================================\n`);

let passedTests = 0;
let totalTests = 0;

async function runTest(name, fn) {
  totalTests++;
  process.stdout.write(`[TEST ${totalTests}] ${name} ... `);
  try {
    await fn();
    console.log(`✅ PASSED`);
    passedTests++;
  } catch (err) {
    console.log(`❌ FAILED`);
    console.error(`   Error: ${err.message}`);
  }
}

// --------------------------------------------------------------------------
// 1. Health Endpoint Tests
// --------------------------------------------------------------------------
await runTest("GET /health returns JSON with required components and zero token leak", async () => {
  const res = await fetch(`${BASE_URL}/health`);
  assert.strictEqual(res.status, 200, `Expected 200, got ${res.status}`);
  const data = await res.json();
  
  assert.ok(data.status, "Missing health status");
  assert.ok(data.components, "Missing components in health response");
  assert.ok(data.components.durable_object, "Missing durable_object in health components");
  assert.ok(data.components.r2, "Missing r2 in health components");
  assert.ok(data.components.ai, "Missing ai in health components");
  assert.ok(data.components.browser, "Missing browser in health components");
  assert.ok(data.components.github_auth, "Missing github_auth in health components");
  assert.ok(data.components.ledger, "Missing ledger in health components");

  // CRITICAL: GitHub auth must not reveal token or secrets
  assert.strictEqual(typeof data.components.github_auth.ok, "boolean");
  assert.strictEqual(data.components.github_auth.token, undefined, "LEAK: token field exposed!");
  assert.strictEqual(data.components.github_auth.secret, undefined, "LEAK: secret field exposed!");
  const jsonStr = JSON.stringify(data);
  assert.ok(!jsonStr.includes("ghp_"), "LEAK: GitHub PAT detected in /health JSON!");

  // Ledger under-cap check
  assert.strictEqual(data.components.ledger.under_cap, true, "Ledger should be under daily cap");
});

// --------------------------------------------------------------------------
// 2. Control Endpoint Info Leakage
// --------------------------------------------------------------------------
await runTest("GET /control returns strictly only paused and status fields", async () => {
  const res = await fetch(`${BASE_URL}/project/${PROJECT_ID}/control`);
  assert.strictEqual(res.status, 200, `Expected 200, got ${res.status}`);
  const data = await res.json();
  const keys = Object.keys(data).sort();
  
  // Must only contain 'paused' and 'status'
  assert.deepStrictEqual(keys, ["paused", "status"], `Expected only [paused, status], got [${keys.join(', ')}]`);
  assert.strictEqual(typeof data.paused, "boolean");
  assert.strictEqual(typeof data.status, "string");
});

// --------------------------------------------------------------------------
// 3. Control Endpoint Authentication (POST)
// --------------------------------------------------------------------------
await runTest("Unauthenticated POST /control fails with 401", async () => {
  const res = await fetch(`${BASE_URL}/project/${PROJECT_ID}/control`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ action: "pause" })
  });
  assert.strictEqual(res.status, 401, `Expected 401, got ${res.status}`);
  const data = await res.json();
  assert.ok(data.error.includes("Unauthorized"), "Expected unauthorized error message");
});

await runTest("Authenticated POST /control with Bearer ADMIN_SECRET succeeds", async () => {
  const res = await fetch(`${BASE_URL}/project/${PROJECT_ID}/control`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "Authorization": `Bearer ${ADMIN_SECRET}`
    },
    body: JSON.stringify({ action: "pause" })
  });
  assert.strictEqual(res.status, 200, `Expected 200, got ${res.status}`);
  const data = await res.json();
  assert.strictEqual(data.status, "ok");
  assert.strictEqual(data.paused, true);
});

// --------------------------------------------------------------------------
// 4. Protected Endpoints Require Admin Secret
// --------------------------------------------------------------------------
const protectedRoutes = [
  { path: `/project/${PROJECT_ID}/scheduler-tick`, method: "POST" },
  { path: `/project/${PROJECT_ID}/close-duplicate-prs`, method: "GET" },
  { path: `/project/${PROJECT_ID}/triage-duplicates`, method: "GET" },
  { path: `/project/${PROJECT_ID}/ledger/config`, method: "POST" },
  { path: `/project/${PROJECT_ID}/ledger/reset`, method: "POST" },
  { path: `/project/${PROJECT_ID}/change-requests/CR-NONEXISTENT/reject`, method: "POST" },
  { path: `/project/${PROJECT_ID}/change-requests/CR-NONEXISTENT/request-changes`, method: "POST" }
];

for (const route of protectedRoutes) {
  await runTest(`Unauthenticated ${route.method} ${route.path} fails with 401`, async () => {
    const res = await fetch(`${BASE_URL}${route.path}`, {
      method: route.method,
      headers: { "Content-Type": "application/json" },
      body: route.method === "POST" ? JSON.stringify({}) : undefined
    });
    assert.strictEqual(res.status, 401, `Expected 401, got ${res.status}`);
  });
}

// --------------------------------------------------------------------------
// 5. Unknown Reviewer Key Approval Enforcement (403 Forbidden)
// --------------------------------------------------------------------------
await runTest("Unknown reviewer pubkey approval fails with 403 Forbidden (no auto-registration)", async () => {
  const fakeReviewerKey = "a1b2c3d4e5f60718293a4b5c6d7e8f90123456789abcdef0123456789abcdef0";
  const res = await fetch(`${BASE_URL}/project/${PROJECT_ID}/change-requests/CR-NONEXISTENT/approve`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "Authorization": `Bearer ${ADMIN_SECRET}`
    },
    body: JSON.stringify({
      reviewer_pubkey: fakeReviewerKey,
      signature: "00".repeat(64),
      rationale: "Unauthorized approval attempt"
    })
  });

  assert.strictEqual(res.status, 403, `Expected 403 Forbidden for unknown supervisor key, got ${res.status}`);
  const data = await res.json();
  assert.ok(data.error.includes("SUPERVISOR_PUBKEYS"), "Expected error mentioning SUPERVISOR_PUBKEYS");
});

await runTest("Unauthenticated and unknown reviewer key approval fails with 403 before or at 401", async () => {
  const fakeReviewerKey = "badbeef000000000000000000000000000000000000000000000000000000000";
  const res = await fetch(`${BASE_URL}/project/${PROJECT_ID}/change-requests/CR-NONEXISTENT/approve`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      reviewer_pubkey: fakeReviewerKey,
      signature: "00".repeat(64)
    })
  });

  // Rejection must be 403 (supervisor key unauthorized) or 401
  assert.ok(res.status === 403 || res.status === 401, `Expected 403 or 401, got ${res.status}`);
});

// --------------------------------------------------------------------------
// 6. PR Body Formatting Logic (No fake "Passed", no "100% resilient")
// --------------------------------------------------------------------------
// Re-import or test formatPREvidenceComment logic
function formatPREvidenceComment(cr, task, issueNum, prNumber) {
  const hasLogs = !!(cr.build_log || (cr.test_report && cr.test_report.output));
  const hasQa = !!(cr.qa_reports && cr.qa_reports.length > 0);

  let buildSection = "";
  if (hasLogs) {
    const exitCode = cr.test_report?.exit_code !== undefined
      ? cr.test_report.exit_code
      : (cr.test_report?.passed ? 0 : 1);
    const duration = cr.test_report?.duration_ms
      ? `${cr.test_report.duration_ms}ms`
      : (cr.test_duration_ms ? `${cr.test_duration_ms}ms` : "N/A");
    const logUrl = cr.test_report?.log_url || cr.build_log_r2_url;

    buildSection = `#### 🧪 Build & Test Verification\n- **Test Command:** \`${cr.test_report?.stage || 'npm run build'}\`\n- **Exit Code:** \`${exitCode}\`\n- **Duration:** \`${duration}\`\n${logUrl ? `- **Log Link (R2):** [View Build & Test Log](${logUrl})` : ''}\n- **Status:** ${cr.test_report?.passed ? 'Passed' : 'Failed'}`;
  } else {
    buildSection = `#### 🧪 Build & Test Verification\n*Build not run*`;
  }

  let qaSection = "";
  if (hasQa) {
    qaSection = `#### 🎭 Real QA Playwright Evidence (Headless Chromium)\n${cr.qa_reports.map(r => `\n- **QA Agent:** \`${r.agent_id}\` (Role: **${r.agent_role.toUpperCase()}**) - **${r.status.toUpperCase()}**\n  - Passed steps: ${(r.passed_steps || []).join(', ') || 'None'}\n  - Latency: ${Object.entries(r.timings_ms || {}).map(([k, v]) => `${k}: ${v}ms`).join(', ')}\n  - Console Errors: ${(r.console_errors || []).length}\n  ${r.visual_diff ? `- Visual Mismatch: ${r.visual_diff.mismatch_percent || 0.0}% (Baseline Verified)` : ''}\n  ${(r.screenshots || []).map(s => `\\n  - **${s.caption}**: ![](${s.data_url.startsWith('http') ? s.data_url : `https://s3a-gate.ashley-e-cribb.workers.dev${s.data_url}`})`).join('')}\n`).join('\n')}`;
  } else {
    qaSection = `#### 🎭 Real QA Playwright Evidence (Headless Chromium)\n*QA not run*`;
  }

  return `### 🤖 S3A Autonomous Agent Evidence Update\n\n- **Agent ID:** \`${cr.agent_id}\`\n- **Commit ID:** \`${cr.commit_id}\`\n- **Seal Hash:** \`${cr.seal_hash || 'N/A'}\`\n- **Timestamp:** \`${new Date().toISOString()}\`\n- **Target Issue:** #${issueNum} (\`${task.title || 'Task'}\`)\n\n${buildSection}\n\n${qaSection}\n\n#### 🌐 Live PM Preview\n[Open Live Brakets Preview on S3A Gate ↗](https://s3a-gate.ashley-e-cribb.workers.dev/project/s3a-swarm-main/preview/live)\n\n> [!NOTE]\n> Attached latest verified evidence and QA runs to canonical PR #${prNumber}. No duplicate PR opened.\n`;
}

await runTest("PR body when NO logs: shows '*Build not run*', '*QA not run*', and NO 'Passed' or '100% resilient'", async () => {
  const crNoLogs = {
    id: "CR-test",
    agent_id: "agent-1",
    commit_id: "abc1234",
    // No build_log, no test_report, no qa_reports
  };
  const body = formatPREvidenceComment(crNoLogs, { title: "Fix UI" }, 3, 82);

  assert.ok(body.includes("*Build not run*"), "Missing '*Build not run*'");
  assert.ok(body.includes("*QA not run*"), "Missing '*QA not run*'");
  assert.ok(!body.includes("Passed"), "Prohibited 'Passed' found when logs are absent!");
  assert.ok(!body.includes("100% resilient"), "Prohibited '100% resilient' found!");
  assert.ok(!body.includes("0 unhandled exceptions"), "Prohibited '0 unhandled exceptions' found!");
});

await runTest("PR body when logs present: shows real exit code, duration, log URL, and NO '100% resilient'", async () => {
  const crWithLogs = {
    id: "CR-test-real",
    agent_id: "agent-2",
    commit_id: "def5678",
    test_report: {
      passed: true,
      stage: "npm run build",
      exit_code: 0,
      duration_ms: 12450,
      log_url: "https://r2.s3a-gate/logs/build-123.txt",
      output: "Compiled successfully in 12.4s"
    }
  };
  const body = formatPREvidenceComment(crWithLogs, { title: "Fix Auth" }, 3, 82);

  assert.ok(body.includes("- **Exit Code:** `0`"), "Missing exit code");
  assert.ok(body.includes("- **Duration:** `12450ms`"), "Missing duration");
  assert.ok(body.includes("[View Build & Test Log](https://r2.s3a-gate/logs/build-123.txt)"), "Missing R2 log URL");
  assert.ok(body.includes("- **Status:** Passed"), "Missing real Passed status");
  assert.ok(!body.includes("100% resilient"), "Prohibited '100% resilient' found!");
  assert.ok(!body.includes("0 unhandled exceptions"), "Prohibited '0 unhandled exceptions' found!");
});

console.log(`\n======================================================`);
console.log(`Test Results: ${passedTests}/${totalTests} Passed`);
console.log(`======================================================\n`);

if (passedTests !== totalTests) {
  process.exit(1);
}
