// scripts/verify-needs-judgment-queue.mjs
// Verifies all requirements for Needs Judgment queue:
// 1. Never list a CR in Needs Judgment if checks failed (build/QA) or commit_sha is reused.
//    Move those to Failed Verification automatically.
// 2. Needs judgment only shows CRs with passing checks, a fresh commit, and a real diff.
//    High risk needs a second human (Ashley + ward).
// 3. Clean current queue: CR-FINDING-CREATEERRORS-8-cui5 -> Failed Verification.

import assert from "node:assert/strict";

const BASE_URL = process.env.GATE_URL || "https://s3a-gate.ashley-e-cribb.workers.dev";
const PROJECT_ID = "s3a-swarm-main";
const ADMIN_SECRET = process.env.ADMIN_SECRET || "s3a-admin-secret-dev";

async function main() {
  console.log("===============================================================================");
  console.log("   VERIFYING NEEDS JUDGMENT QUEUE & VERIFICATION ENFORCEMENT                   ");
  console.log(`   Target: ${BASE_URL} (Project: ${PROJECT_ID})                                `);
  console.log("===============================================================================\n");

  // Step 1: Query change-requests to verify automatic sanitation of current queue
  console.log("1. Querying GET /project/:projectId/change-requests to trigger queue cleaning...");
  const crRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/change-requests`);
  assert.equal(crRes.status, 200, `GET change-requests failed: ${crRes.status}`);
  const crData = await crRes.json();
  const changeRequests = crData.change_requests || [];

  // Step 2: Verify CR-FINDING-CREATEERRORS-8-cui5 is moved to Failed Verification
  console.log("2. Checking CR-FINDING-CREATEERRORS-8-cui5 status...");
  const crCreateErrors = changeRequests.find(c => c.id === "CR-FINDING-CREATEERRORS-8-cui5");
  assert.ok(crCreateErrors, "CR-FINDING-CREATEERRORS-8-cui5 must exist in change requests");
  assert.equal(
    crCreateErrors.status,
    "failed_verification",
    `CR-FINDING-CREATEERRORS-8-cui5 must have status 'failed_verification', got '${crCreateErrors.status}'`
  );
  assert.ok(
    crCreateErrors.human_rationale && crCreateErrors.human_rationale.includes("Failed Verification"),
    "CR-FINDING-CREATEERRORS-8-cui5 must have explanatory Failed Verification rationale"
  );
  console.log("   ✓ CR-FINDING-CREATEERRORS-8-cui5 successfully moved to Failed Verification.");

  // Step 3: Verify that NO CR in Needs Judgment has failing checks or reused commit SHA
  console.log("3. Verifying that NO CR in Needs Judgment has failed checks or reused commit_sha...");
  const shaCounts = new Map();
  for (const cr of changeRequests) {
    const sha = (cr.commit_sha || cr.commit_id || "").trim();
    if (sha && sha !== "not run") {
      shaCounts.set(sha, (shaCounts.get(sha) || 0) + 1);
    }
  }

  const needsJudgmentQueue = changeRequests.filter(c => c.status === "pending_review" || c.status === "changes_requested");
  console.log(`   Total CRs in pending_review/changes_requested: ${needsJudgmentQueue.length}`);

  for (const cr of needsJudgmentQueue) {
    const testExitCode = cr.test_report?.test_exit_code ?? cr.test_report?.exit_code;
    const buildPassed = cr.build_failed !== true && (cr.test_report?.passed !== false) && (testExitCode === undefined || testExitCode === 0);
    assert.ok(buildPassed, `CR #${cr.id} in Needs Judgment must have passed build!`);

    const qaFailed = cr.qa_status === "failed" || (cr.qa_reports && cr.qa_reports.some(q => q.status === "failed"));
    assert.ok(!qaFailed, `CR #${cr.id} in Needs Judgment must not have failed QA!`);

    const sha = (cr.commit_sha || cr.commit_id || "").trim();
    assert.ok(sha && sha !== "not run", `CR #${cr.id} in Needs Judgment must have a valid commit_sha`);

    // Check reused sha across different tasks or older CRs
    const isReused = changeRequests.some(other =>
      other.id !== cr.id &&
      (other.commit_sha === sha || other.commit_id === sha) &&
      (other.task_id !== cr.task_id || (other.created_at || 0) < (cr.created_at || 0))
    );
    assert.ok(!isReused, `CR #${cr.id} in Needs Judgment must have a fresh, non-reused commit_sha!`);

    const diff = (cr.diff || "").trim();
    assert.ok(diff && diff.length > 0 && !diff.includes("(No file changes diff attached)"), `CR #${cr.id} must have a real diff!`);
  }
  console.log("   ✓ All CRs in Needs Judgment strictly satisfy quality criteria (passing checks, fresh commit, real diff).");

  // Step 4: Verify UI HTML for Pulls page excludes CR-FINDING-CREATEERRORS-8-cui5 from Needs Judgment
  console.log("4. Verifying /p/:projectId/pulls HTML...");
  const pullsRes = await fetch(`${BASE_URL}/p/${PROJECT_ID}/pulls`);
  assert.equal(pullsRes.status, 200);
  const pullsHtml = await pullsRes.text();

  assert.ok(
    !pullsHtml.includes(`href="/p/${PROJECT_ID}/pulls/CR-FINDING-CREATEERRORS-8-cui5"`),
    "CR-FINDING-CREATEERRORS-8-cui5 must NOT appear in default Needs Judgment tab list"
  );
  console.log("   ✓ CR-FINDING-CREATEERRORS-8-cui5 is strictly excluded from default Needs Judgment view.");

  // Step 5: Test submission of an invalid CR (failing build exit code)
  console.log("5. Testing submission barrier: submitting CR with failed build checks...");
  const invalidCr = {
    id: `CR-INVALID-TEST-${Date.now().toString(36)}`,
    task_id: "TASK-TEST-FAIL",
    agent_id: "gate-agent",
    agent_pubkey: "0a9ad8193ed7acb03a4f795bf83835d1d0ed9b6a1c42bb91114379358a91c38a",
    title: "Test invalid CR with failing build",
    commit_id: `c_invalid_${Date.now()}`,
    commit_sha: `c_invalid_${Date.now()}`,
    seal_hash: "e764188bbdbc9ae8cfaa59b9bff83da7a11d3fa8b5ca78c5fb17447183967fab",
    message: "Failing test commit",
    diff: "+++ b/test.ts\n+ const a = 1;",
    test_report: {
      passed: false,
      exit_code: 1,
      output: "error TS2304: Cannot find name 'foo'",
    },
    touched_files: ["test.ts"],
    risk_level: "low",
    requires_human: true,
    status: "pending_review",
    created_at: Date.now(),
  };

  const submitRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/change-requests`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(invalidCr),
  });
  assert.equal(submitRes.status, 400, "Submitting CR with failing build checks must be rejected with 400");
  const submitData = await submitRes.json();
  assert.equal(submitData.status, "failed_verification", "Status must be failed_verification");
  console.log("   ✓ Invalid CR submission was blocked and automatically set to failed_verification.");

  // Step 6: Test submission of CR reusing an existing commit SHA
  console.log("6. Testing submission barrier: submitting CR with reused commit_sha...");
  const reusedCr = {
    id: `CR-REUSED-TEST-${Date.now().toString(36)}`,
    task_id: "TASK-TEST-REUSE",
    agent_id: "gate-agent",
    agent_pubkey: "0a9ad8193ed7acb03a4f795bf83835d1d0ed9b6a1c42bb91114379358a91c38a",
    title: "Test invalid CR with reused commit SHA",
    commit_id: "3df6448db26378db3c69e68ecec2b843c102c1cb", // Existing SHA
    commit_sha: "3df6448db26378db3c69e68ecec2b843c102c1cb",
    seal_hash: "e764188bbdbc9ae8cfaa59b9bff83da7a11d3fa8b5ca78c5fb17447183967fab",
    message: "Reused SHA commit",
    diff: "+++ b/test.ts\n+ const b = 2;",
    test_report: {
      passed: true,
      exit_code: 0,
      output: "build passed",
    },
    touched_files: ["test.ts"],
    risk_level: "low",
    requires_human: true,
    status: "pending_review",
    created_at: Date.now(),
  };

  const reuseRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/change-requests`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(reusedCr),
  });
  assert.equal(reuseRes.status, 400, "Submitting CR with reused commit SHA must be rejected with 400");
  const reuseData = await reuseRes.json();
  assert.equal(reuseData.status, "failed_verification", "Status must be failed_verification");
  console.log("   ✓ Reused commit_sha submission was blocked and automatically set to failed_verification.");

  // Step 7: Verify High-Risk two-human requirement display (Ashley + ward)
  console.log("7. Verifying High-Risk second-human requirement on detail and listing views...");
  const detailRes = await fetch(`${BASE_URL}/p/${PROJECT_ID}/pulls/CR-FINDING-TOURNAMENT-138-ydzq`);
  assert.equal(detailRes.status, 200);
  const detailHtml = await detailRes.text();
  assert.ok(
    detailHtml.includes("Ashley") && detailHtml.includes("Ward"),
    "Detail view for sensitive/high-risk items must mention Ashley + Ward supervisory requirements"
  );
  console.log("   ✓ High risk second-human requirement (Ashley + ward) verified.");

  console.log("\n===============================================================================");
  console.log("   ALL NEEDS JUDGMENT QUEUE VERIFICATION CHECKS PASSED!                       ");
  console.log("===============================================================================\n");
}

main().catch(err => {
  console.error("FATAL verification error:", err);
  process.exit(1);
});
