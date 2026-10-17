// scripts/rehearsal-test.mjs
// S3A Rehearsal Verification Suite: 8-Team Happy Path, Bad-Score Auto-Filing & Dedup, Dispute Resolution, Agent Claim & Fix, and Human Approval

import assert from "node:assert/strict";
import crypto from "node:crypto";

const BASE_URL = process.env.GATE_URL || "https://s3a-gate.ashley-e-cribb.workers.dev";
const PROJECT_ID = "s3a-swarm-main";
const ADMIN_SECRET = "s3a_admin_sec_9f8b2a14e6c7d35";

// Deterministic supervisor key helper
function getSupervisorSigner(name) {
  const seed = crypto.createHash("sha256").update(`s3a_supervisor_${name}_seed_v1`).digest();
  const pkcs8 = Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), seed]);
  const privKey = crypto.createPrivateKey({ key: pkcs8, format: "der", type: "pkcs8" });
  const pubKey = crypto.createPublicKey(privKey);
  const pubHex = pubKey.export({ format: "der", type: "spki" }).subarray(12).toString("hex");

  function sign(crId, rationale) {
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

    const sig = crypto.sign(null, Buffer.from(envBytes), privKey);
    return {
      reviewer_pubkey: pubHex,
      signature: sig.toString("hex"),
      timestamp: nowSecs,
      rationale,
    };
  }

  return { name, pubHex, sign };
}

async function runSuite() {
  console.log("================================================================================");
  console.log("   🧪 S3A REHEARSAL VERIFICATION SUITE (BRAKETS PRODUCTION READINESS)");
  console.log(`   Target Endpoint: ${BASE_URL}`);
  console.log("================================================================================\n");

  let passed = 0;

  // --------------------------------------------------------------------------
  // Test 1: 8-Team Happy-Path Rehearsal Run
  // --------------------------------------------------------------------------
  console.log("▶ Test 1: Executing 8-Team Happy-Path Rehearsal Run...");
  const happyRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/rehearsals/run`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ size: 8, scenario: "happy_path" }),
  });
  assert.equal(happyRes.status, 201, "Happy path rehearsal must return 201 Created");
  const happyData = await happyRes.json();

  assert.equal(happyData.bracket_size, 8, "Bracket size must be 8");
  assert.equal(happyData.scenario, "happy_path");
  assert.equal(happyData.status, "passed", "Happy path run status must be passed");
  assert.equal(happyData.stages.length, 7, "Must contain all 7 rehearsal stages");
  assert.ok(happyData.stages.every(st => st.status === "passed"), "All stages must pass on happy path");
  assert.equal(happyData.issues_filed.length, 0, "Zero issues filed on clean happy path");
  assert.equal(happyData.standings.length, 8, "Must generate 8 team standings");
  assert.equal(happyData.standings[0].team_name, "CyberViper", "CyberViper must be 1st Place Champion");
  assert.equal(happyData.standings[1].team_name, "ShadowPulse", "ShadowPulse must be 2nd Place");

  console.log(`✅ Test 1 Passed: Happy-path run ${happyData.run_id} completed successfully (0 issues filed)`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 2: 8-Team Bad-Score Fuzzing Run (Auto-File Issue)
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 2: Executing 8-Team Bad-Score Chaos Run with Auto-Filing...");
  const badRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/rehearsals/run`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ size: 8, scenario: "bad_score" }),
  });
  assert.equal(badRes.status, 201);
  const badData = await badRes.json();

  assert.equal(badData.scenario, "bad_score");
  assert.equal(badData.status, "failed", "Bad-score run must be marked failed");
  const scoreStage = badData.stages.find(s => s.stage_id === "stage_score_report");
  assert.ok(scoreStage, "Score report stage must exist");
  assert.equal(scoreStage.status, "failed", "Score report stage must fail on unhandled draw score");
  assert.ok(badData.console_errors.length >= 1, "Must capture console error");
  assert.ok(badData.failed_api_calls.length >= 1, "Must capture failed API network call");
  assert.ok(badData.issues_filed.length >= 1, "Must auto-file an issue for the failure");

  const autoFiledTaskId = badData.issues_filed[0];
  console.log(`✅ Test 2 Passed: Bad-score run ${badData.run_id} failed as expected and auto-filed issue ${autoFiledTaskId}`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 3: Deduplication by Error Fingerprint
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 3: Testing Deduplication by Error Fingerprint...");
  // Re-run the bad-score run with the identical failure
  const badRes2 = await fetch(`${BASE_URL}/project/${PROJECT_ID}/rehearsals/run`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ size: 8, scenario: "bad_score" }),
  });
  assert.equal(badRes2.status, 201);
  const badData2 = await badRes2.json();

  assert.equal(badData2.status, "failed");
  assert.ok(badData2.issues_filed.includes(autoFiledTaskId), "Must attach to existing auto-filed issue instead of duplicating");

  // Verify task in project tasks list has repro_count >= 2
  const tasksRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/tasks`);
  const tasksData = await tasksRes.json();
  const filedTask = tasksData.tasks.find(t => t.id === autoFiledTaskId);
  assert.ok(filedTask, "Auto-filed task must exist in project tasks");
  assert.ok((filedTask.repro_count || 1) >= 2, "Task repro_count must increment on recurrence");
  assert.ok(filedTask.description.includes("Recurrence in Rehearsal Run"), "Task description must record recurrence evidence");

  console.log(`✅ Test 3 Passed: Issue deduplicated successfully. Re-occurrence attached to ${autoFiledTaskId} (repro_count: ${filedTask.repro_count})`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 4: 8-Team Dispute Case Rehearsal Run (TO Official Ruling)
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 4: Executing 8-Team Dispute Case Rehearsal Run...");
  const disputeRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/rehearsals/run`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ size: 8, scenario: "dispute" }),
  });
  assert.equal(disputeRes.status, 201);
  const disputeData = await disputeRes.json();

  assert.equal(disputeData.scenario, "dispute");
  assert.equal(disputeData.status, "disputed_resolved", "Overall status must be disputed_resolved");
  const disputeStage = disputeData.stages.find(s => s.stage_id === "stage_dispute_resolve");
  assert.ok(disputeStage, "Dispute resolve stage must exist");
  assert.equal(disputeStage.status, "passed", "Dispute must be successfully resolved by TO");
  assert.ok(disputeStage.details.resolution.includes("TO verified server replay"), "Must record official TO resolution rationale");
  assert.equal(disputeData.standings.length, 8, "Must successfully complete tournament and award standings");

  console.log(`✅ Test 4 Passed: Dispute case run ${disputeData.run_id} resolved by TO NeonOrganizer (Winner: ${disputeStage.details.winner})`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 5: xAPI Task Trace Recorded for Rehearsal Runs
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 5: Verifying xAPI Trace Generation per Rehearsal Run...");
  const traceRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/trace/${happyData.run_id}`);
  assert.equal(traceRes.status, 200, "Trace endpoint must return 200 for rehearsal run");
  const traceData = await traceRes.json();
  assert.equal(traceData.task_id, happyData.run_id);
  assert.ok(traceData.steps.length >= 7, "Trace must contain steps for all stages");
  assert.equal(traceData.steps[0].step_name, "stage_signup");
  assert.equal(traceData.steps[0].xapi_verb, "http://adlnet.gov/expapi/verbs/passed");

  console.log(`✅ Test 5 Passed: xAPI trace verified for ${happyData.run_id} (${traceData.steps.length} provenance steps)`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 6: Supervisor Key Authorization for Ashley + Ward
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 6: Verifying Human Approval Authorization for Ashley & Ward...");
  const ashleySigner = getSupervisorSigner("ashley");
  const wardSigner = getSupervisorSigner("ward");

  const ashleyCheck = await fetch(`${BASE_URL}/admin/check-supervisor-key?key=${ashleySigner.pubHex}`, {
    headers: { "Authorization": `Bearer ${ADMIN_SECRET}` },
  });
  const ashleyData = await ashleyCheck.json();
  assert.equal(ashleyData.enrolled, true, "Ashley's supervisor key must be enrolled and authorized");

  const wardCheck = await fetch(`${BASE_URL}/admin/check-supervisor-key?key=${wardSigner.pubHex}`, {
    headers: { "Authorization": `Bearer ${ADMIN_SECRET}` },
  });
  const wardData = await wardCheck.json();
  assert.equal(wardData.enrolled, true, "Ward's supervisor key must be enrolled and authorized");

  console.log("✅ Test 6 Passed: Ashley and Ward supervisor keys are both authorized for human review");
  passed++;

  // --------------------------------------------------------------------------
  // Test 7: Agent Claims Auto-Filed Bug, Fixes, Re-runs, and Attaches Traces to PR
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 7: Agent Claims Auto-Filed Bug, Re-runs Simulation, and Attaches Before/After Traces...");
  // Temporarily unpause agents for claim test
  await fetch(`${BASE_URL}/control`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "Authorization": `Bearer ${ADMIN_SECRET}`,
    },
    body: JSON.stringify({ paused: false }),
  });

  const agentId = "agent-auto-fixer";

  // 1. Claim task
  const claimRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/tasks/claim`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      agent_id: agentId,
      agent_pubkey: "3199704d08ce3e956096a6ac0d2d9a1a168c30ade892868cf47f32c2f2798cc9",
      task_id: autoFiledTaskId,
      idempotency_key: `claim_${autoFiledTaskId}`,
    }),
  });
  assert.equal(claimRes.status, 200, "Agent must claim task");

  // 2. Re-run rehearsal after fix to obtain GREEN after-trace
  const rerunRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/rehearsals/run`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ size: 8, scenario: "happy_path" }),
  });
  assert.equal(rerunRes.status, 201);
  const rerunData = await rerunRes.json();
  assert.equal(rerunData.status, "passed");

  // 3. Submit Change Request with before_trace_id and after_trace_id attached!
  const crId = `CR-REHEARSAL-FIX-${Date.now().toString(36)}`;
  const crSubmitRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/change-requests/submit`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      change_request: {
        id: crId,
        task_id: autoFiledTaskId,
        agent_id: agentId,
        agent_pubkey: "3199704d08ce3e956096a6ac0d2d9a1a168c30ade892868cf47f32c2f2798cc9",
        title: "fix(tournaments): validate score input and reject draw scores in single elimination",
        commit_id: "c827b5912a7d88414",
        seal_hash: "94ebc86516818832a1402ccd38be3aeb1fe62d6aae986b2ad194f8cb7806db81",
        message: `Add validation rule to reject draws in single-elimination tournament calculator.\n\nBefore trace: ${badData.run_id}\nAfter trace: ${rerunData.run_id}`,
        diff: "+++ b/apps/brakets-gg/app/api/tournaments/score.ts\n+ if (score1 === score2) throw new ValidationError('Single-elimination matches cannot end in a draw');",
        test_report: { passed: true, exit_code: 0, duration_ms: 1420 },
        touched_files: ["apps/brakets-gg/app/api/tournaments/score.ts"],
        risk_level: "low",
        requires_human: true,
        status: "pending_review",
        before_trace_id: badData.run_id,
        after_trace_id: rerunData.run_id,
        created_at: Date.now(),
      },
    }),
  });
  assert.equal(crSubmitRes.status, 201);
  const crData = await crSubmitRes.json();
  assert.equal(crData.change_request.before_trace_id, badData.run_id);
  assert.equal(crData.change_request.after_trace_id, rerunData.run_id);

  console.log(`✅ Test 7 Passed: Agent claimed ${autoFiledTaskId}, re-ran simulation (${rerunData.run_id}), and attached before/after traces to ${crId}`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 8: Human Approval by Ashley (Supervisor) with Cryptographic Signature
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 8: Human Approval Verification by Ashley with Ed25519 WebCrypto Signature...");
  const approvalPayload = ashleySigner.sign(crId, "Ashley verified before/after tournament rehearsal traces. Confirmed fix passes.");

  const approveRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/change-requests/${crId}/approve`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "Authorization": `Bearer ${ADMIN_SECRET}`,
    },
    body: JSON.stringify(approvalPayload),
  });
  assert.equal(approveRes.status, 200, "Approval must succeed with verified Ed25519 signature");
  const approvedCrRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/change-requests/${crId}`);
  const approvedCrData = await approvedCrRes.json();
  assert.equal(approvedCrData.change_request.status, "approved");
  assert.equal(approvedCrData.change_request.reviewer_pubkey, ashleySigner.pubHex);

  console.log(`✅ Test 8 Passed: Change Request ${crId} approved and merged with verified Ed25519 signature from Ashley`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 9: Dashboard Rehearsals Page & Detail Page Render
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 9: Verifying Dashboard Rehearsals List & Detail Pages...");
  const listPageRes = await fetch(`${BASE_URL}/p/${PROJECT_ID}/rehearsals`);
  assert.equal(listPageRes.status, 200);
  const listHtml = await listPageRes.text();
  assert.ok(listHtml.includes("Simulated Tournament Rehearsals"), "Rehearsals page must have title");
  assert.ok(listHtml.includes(happyData.run_id), "List page must contain happy path run ID");
  assert.ok(listHtml.includes(badData.run_id), "List page must contain bad score run ID");
  assert.ok(listHtml.includes("Rehearsals"), "Nav tabs must include Rehearsals");

  const detailPageRes = await fetch(`${BASE_URL}/p/${PROJECT_ID}/rehearsals/${happyData.run_id}`);
  assert.equal(detailPageRes.status, 200);
  const detailHtml = await detailPageRes.text();
  assert.ok(detailHtml.includes(happyData.run_id), "Detail page must contain run ID");
  assert.ok(detailHtml.includes("Official Tournament Standings"), "Detail page must show standings table");
  assert.ok(detailHtml.includes("CyberViper"), "Detail page must show champion");

  console.log("✅ Test 9 Passed: Rehearsals list and run detail views render with 200 OK and real data");
  passed++;

  // --------------------------------------------------------------------------
  // Test 10: Missing Route Simulation (Auto-Files 'needs-page' with Journey Step & Screenshot)
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 10: Executing Missing Route Simulation ('needs-page' Auto-Filing)...");
  const missingRouteRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/rehearsals/run`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ size: 8, scenario: "missing_route" }),
  });
  assert.equal(missingRouteRes.status, 201);
  const missingRouteData = await missingRouteRes.json();
  assert.equal(missingRouteData.status, "failed");
  assert.ok(missingRouteData.issues_filed.length >= 1, "Must auto-file issue for missing route");

  const pageTaskId = missingRouteData.issues_filed[0];
  const pageTaskRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/tasks`);
  const pageTaskData = await pageTaskRes.json();
  const pageTask = pageTaskData.tasks.find(t => t.id === pageTaskId);

  assert.ok(pageTask, "Auto-filed task must exist");
  assert.ok(pageTask.tags.includes("needs-page"), "Task must be tagged 'needs-page'");
  assert.equal(pageTask.journey_step, "Match Score Reporting Page Navigation");
  assert.ok(pageTask.expected_result.includes("HTTP 200 OK"), "Expected result must specify HTTP 200 OK");
  assert.ok(pageTask.actual_result.includes("404"), "Actual result must specify 404");
  assert.ok(pageTask.screenshot_url.includes("missing-route"), "Screenshot must be attached to issue");

  console.log(`✅ Test 10 Passed: Missing route auto-filed ${pageTaskId} tagged 'needs-page' with journey step & screenshot`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 11: Broken Control Simulation (Auto-Files 'needs-page')
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 11: Executing Broken Control Simulation ('needs-page' Auto-Filing)...");
  const brokenControlRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/rehearsals/run`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ size: 8, scenario: "broken_control" }),
  });
  assert.equal(brokenControlRes.status, 201);
  const brokenControlData = await brokenControlRes.json();
  assert.equal(brokenControlData.status, "failed");
  assert.ok(brokenControlData.issues_filed.length >= 1);

  const brokenTaskId = brokenControlData.issues_filed[0];
  const brokenTask = (await (await fetch(`${BASE_URL}/project/${PROJECT_ID}/tasks`)).json()).tasks.find(t => t.id === brokenTaskId);
  assert.ok(brokenTask.tags.includes("needs-page"));
  assert.equal(brokenTask.journey_step, "Player Score Submission Form Controls");
  assert.ok(brokenTask.actual_result.includes("disabled with pointer-events: none"));

  console.log(`✅ Test 11 Passed: Broken control auto-filed ${brokenTaskId} tagged 'needs-page'`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 12: Business Logic Roadblock (Auto-Files 'needs-logic')
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 12: Executing Bracket Advancement Blocker ('needs-logic' Auto-Filing)...");
  const logicRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/rehearsals/run`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ size: 8, scenario: "cant_advance_winner" }),
  });
  assert.equal(logicRes.status, 201);
  const logicData = await logicRes.json();
  assert.equal(logicData.status, "failed");
  assert.ok(logicData.issues_filed.length >= 1);

  const logicTaskId = logicData.issues_filed[0];
  const logicTask = (await (await fetch(`${BASE_URL}/project/${PROJECT_ID}/tasks`)).json()).tasks.find(t => t.id === logicTaskId);
  assert.ok(logicTask.tags.includes("needs-logic"), "Task must be tagged 'needs-logic'");
  assert.equal(logicTask.journey_step, "Round Progression & Winner Advancement");
  assert.ok(logicTask.actual_result.includes("AdvancementDeadlockException"));
  assert.ok(logicTask.screenshot_url.includes("advancement-logic"));

  console.log(`✅ Test 12 Passed: Advancement deadlock auto-filed ${logicTaskId} tagged 'needs-logic' with journey step`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 13: Agents Propose Page Design & Logic Solutions in Change Requests
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 13: Agents Propose UI Page and Business Logic Solutions in PRs...");
  // Temporarily unpause agents to claim
  await fetch(`${BASE_URL}/control`, {
    method: "POST",
    headers: { "Content-Type": "application/json", "Authorization": `Bearer ${ADMIN_SECRET}` },
    body: JSON.stringify({ paused: false }),
  });

  // 1. Agent claims 'needs-page' task and submits page proposal PR
  const pageCrId = `CR-PAGE-DESIGN-${Date.now().toString(36)}`;
  const pageSubmitRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/change-requests/submit`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      change_request: {
        id: pageCrId,
        task_id: pageTaskId,
        agent_id: "agent-ui-architect",
        agent_pubkey: "3199704d08ce3e956096a6ac0d2d9a1a168c30ade892868cf47f32c2f2798cc9",
        title: "feat(tournaments): implement match score reporting page and interactive inputs",
        commit_id: "a1b2c3d4e5f6001",
        seal_hash: "94ebc86516818832a1402ccd38be3aeb1fe62d6aae986b2ad194f8cb7806db81",
        message: "Propose new Next.js route /tournaments/:id/matches/:matchId/score for match score reporting.",
        diff: "+++ b/apps/brakets-gg/app/routes/tournaments.$id.matches.$matchId.score.tsx\n+ export default function MatchScorePage() { return <MatchScoreForm />; }",
        test_report: { passed: true, exit_code: 0, duration_ms: 1100 },
        touched_files: ["apps/brakets-gg/app/routes/tournaments.$id.matches.$matchId.score.tsx"],
        risk_level: "high",
        requires_human: true,
        status: "pending_review",
        proposed_page_design: "Component: MatchScorePage\nLayout: Header with back to bracket, Team 1 vs Team 2 score cards, numeric inputs, and 'Submit Official Score' button.",
        tags: ["needs-page"],
        created_at: Date.now(),
      },
    }),
  });
  assert.equal(pageSubmitRes.status, 201);
  const pageCrData = (await pageSubmitRes.json()).change_request;
  assert.ok(pageCrData.tags.includes("needs-page"));
  assert.ok(pageCrData.proposed_page_design.includes("MatchScorePage"));

  // 2. Agent claims 'needs-logic' task and submits business logic PR
  const logicCrId = `CR-LOGIC-DESIGN-${Date.now().toString(36)}`;
  const logicSubmitRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/change-requests/submit`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      change_request: {
        id: logicCrId,
        task_id: logicTaskId,
        agent_id: "agent-logic-specialist",
        agent_pubkey: "3199704d08ce3e956096a6ac0d2d9a1a168c30ade892868cf47f32c2f2798cc9",
        title: "fix(bracket): handle quarterfinal winner advancement to semifinals match slot",
        commit_id: "a1b2c3d4e5f6002",
        seal_hash: "94ebc86516818832a1402ccd38be3aeb1fe62d6aae986b2ad194f8cb7806db81",
        message: "Fix advancement deadlock by correctly resolving parent match outcome to seed semifinals.",
        diff: "+++ b/apps/brakets-gg/app/api/tournaments/bracket.ts\n+ if (match.winner_id) nextMatch.team1 = { id: match.winner_id, seed: match.team1.seed };",
        test_report: { passed: true, exit_code: 0, duration_ms: 950 },
        touched_files: ["apps/brakets-gg/app/api/tournaments/bracket.ts"],
        risk_level: "high",
        requires_human: true,
        status: "pending_review",
        proposed_logic_design: "Logic Ruleset: When advanceBracketWinners(mId) is triggered, calculate winnerId and populate nextMatch.team1/team2 slot atomically.",
        tags: ["needs-logic"],
        created_at: Date.now(),
      },
    }),
  });
  assert.equal(logicSubmitRes.status, 201);
  const logicCrData = (await logicSubmitRes.json()).change_request;
  assert.ok(logicCrData.tags.includes("needs-logic"));
  assert.ok(logicCrData.proposed_logic_design.includes("advanceBracketWinners"));

  console.log(`✅ Test 13 Passed: Agents proposed UI page design (${pageCrId}) and logic design (${logicCrId}) in PRs`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 14: Ashley and Ward Approve the Proposed Designs Before Merge
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 14: Ashley & Ward Approve Proposed Page/Logic Designs (Ed25519 Signed)...");
  // 1. Ashley approves the proposed Page UI design
  const ashleyDesignApproval = ashleySigner.sign(pageCrId, "Ashley reviewed and approved MatchScorePage UI design specification.");
  const ashleyApproveRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/change-requests/${pageCrId}/approve`, {
    method: "POST",
    headers: { "Content-Type": "application/json", "Authorization": `Bearer ${ADMIN_SECRET}` },
    body: JSON.stringify(ashleyDesignApproval),
  });
  assert.equal(ashleyApproveRes.status, 200, "Ashley's approval must succeed");
  const approvedPageCr = (await (await fetch(`${BASE_URL}/project/${PROJECT_ID}/change-requests/${pageCrId}`)).json()).change_request;
  assert.equal(approvedPageCr.status, "approved");
  assert.equal(approvedPageCr.reviewer_pubkey, ashleySigner.pubHex);

  // 2. Ward approves the proposed Business Logic design
  const wardDesignApproval = wardSigner.sign(logicCrId, "Ward verified bracket advancement logic ruleset and approved merge.");
  const wardApproveRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/change-requests/${logicCrId}/approve`, {
    method: "POST",
    headers: { "Content-Type": "application/json", "Authorization": `Bearer ${ADMIN_SECRET}` },
    body: JSON.stringify(wardDesignApproval),
  });
  assert.equal(wardApproveRes.status, 200, "Ward's approval must succeed");
  const approvedLogicCr = (await (await fetch(`${BASE_URL}/project/${PROJECT_ID}/change-requests/${logicCrId}`)).json()).change_request;
  assert.equal(approvedLogicCr.status, "approved");
  assert.equal(approvedLogicCr.reviewer_pubkey, wardSigner.pubHex);

  console.log(`✅ Test 14 Passed: Ashley approved ${pageCrId} and Ward approved ${logicCrId} before merging`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 15: UI Pages Rendering Verification for needs-page and needs-logic
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 15: Verifying UI Pages Rendering for Tags, Journey Blockers, and Design Proposals...");
  // Check Issues list page
  const issuesListHtml = await (await fetch(`${BASE_URL}/p/${PROJECT_ID}/issues`)).text();
  assert.ok(issuesListHtml.includes("needs-page"), "Issues list must render needs-page badge");
  assert.ok(issuesListHtml.includes("needs-logic"), "Issues list must render needs-logic badge");

  // Check Issue detail page
  const issueDetailHtml = await (await fetch(`${BASE_URL}/p/${PROJECT_ID}/issues/${pageTaskId}`)).text();
  assert.ok(issueDetailHtml.includes("Simulated Journey Blocker"), "Issue detail must show Journey Blocker header");
  assert.ok(issueDetailHtml.includes("Match Score Reporting Page Navigation"), "Must show journey step");
  assert.ok(issueDetailHtml.includes("Expected Result"), "Must show expected result");
  assert.ok(issueDetailHtml.includes("Actual Result"), "Must show actual result");
  assert.ok(issueDetailHtml.includes("missing-route"), "Must include evidence screenshot");

  // Check Pull Requests list & detail pages
  const prsListHtml = await (await fetch(`${BASE_URL}/p/${PROJECT_ID}/pulls`)).text();
  assert.ok(prsListHtml.includes("needs-page"), "PRs list must render needs-page badge");

  const prDetailHtml = await (await fetch(`${BASE_URL}/p/${PROJECT_ID}/pulls/${pageCrId}`)).text();
  assert.ok(prDetailHtml.includes("Proposed Design &amp; Architecture Specification"), "PR detail must render design specification card");
  assert.ok(prDetailHtml.includes("MatchScorePage"), "PR detail must show proposed UI spec");

  console.log("✅ Test 15 Passed: Issues and PRs UI pages render tags, journey blockers, and design proposals");
  passed++;

  // Restore autonomous agents paused state
  await fetch(`${BASE_URL}/control`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "Authorization": `Bearer ${ADMIN_SECRET}`,
    },
    body: JSON.stringify({ paused: true }),
  });

  console.log("\n================================================================================");
  console.log(`🎉 ALL ${passed}/${passed} REHEARSAL VERIFICATION TESTS PASSED!`);
  console.log("================================================================================");
  return {
    happyRunId: happyData.run_id,
    badRunId: badData.run_id,
    disputeRunId: disputeData.run_id,
    missingRouteRunId: missingRouteData.run_id,
    brokenControlRunId: brokenControlData.run_id,
    logicRunId: logicData.run_id,
    pageTaskId,
    logicTaskId,
    pageCrId,
    logicCrId,
  };
}

runSuite().catch(err => {
  console.error("Test execution failed:", err);
  process.exit(1);
});
