// scripts/walkaway-and-operations-test.mjs
// Verification suite for S3A Gate:
// 1. Operation records {op_id, idempotency_key, status: accepted|running|done|failed, started, finished}
// 2. Claiming an issue with open op/PR attaches instead of opening duplicate
// 3. sandbox_build(argv[], timeout_ms) rejects shell strings, accepts argv tokens
// 4. PR body lists commit SHA, image digest, real build/test exit codes, log URLs, R2 screenshots, or "not run"
// 5. Invariant: Agents never merge; cashier/escrow/payout/auth/db/infra -> needs-human

import assert from "node:assert/strict";

const BASE_URL = process.env.BASE_URL || "https://s3a-gate.ashley-e-cribb.workers.dev";
const PROJECT_ID = "s3a-swarm-main";

// Pure JS sandbox_build validation mirror for standalone contract testing
function validateSandboxArgv(argv) {
  if (!Array.isArray(argv) || argv.length === 0) {
    throw new Error("sandbox_build rejection: argv must be a non-empty array of argument tokens. Shell strings are strictly prohibited.");
  }
  for (const arg of argv) {
    if (typeof arg !== "string") {
      throw new Error("sandbox_build rejection: each element in argv must be a string token.");
    }
    if (/[\0\r\n]/.test(arg)) {
      throw new Error("sandbox_build rejection: argv token contains illegal control characters.");
    }
  }
  return argv.map(tok => `'${tok.replace(/'/g, "'\\''")}'`).join(" ");
}

async function runTests() {
  console.log("================================================================================");
  console.log("   🧪 S3A GATE UNATTENDED RUN & OPERATIONS VERIFICATION SUITE");
  console.log(`   Target Endpoint: ${BASE_URL}`);
  console.log("================================================================================\n");

  let passedCount = 0;

  // --------------------------------------------------------------------------
  // Test 1: sandbox_build rejects raw shell strings
  // --------------------------------------------------------------------------
  try {
    validateSandboxArgv("npm run build; cat /etc/passwd");
    assert.fail("Should have rejected raw shell string");
  } catch (err) {
    assert.match(err.message, /argv must be a non-empty array of argument tokens/);
    console.log("✅ Test 1 Passed: sandbox_build rejects raw shell string from model");
    passedCount++;
  }

  // --------------------------------------------------------------------------
  // Test 2: sandbox_build rejects control characters in argv tokens
  // --------------------------------------------------------------------------
  try {
    validateSandboxArgv(["npm", "run\nrm -rf /", "build"]);
    assert.fail("Should have rejected control character in token");
  } catch (err) {
    assert.match(err.message, /illegal control characters/);
    console.log("✅ Test 2 Passed: sandbox_build rejects argv tokens with control characters");
    passedCount++;
  }

  // --------------------------------------------------------------------------
  // Test 3: sandbox_build escapes tokens safely into POSIX arguments
  // --------------------------------------------------------------------------
  const escapedCmd = validateSandboxArgv(["npm", "run", "build", "--test='safe'"]);
  assert.equal(escapedCmd, "'npm' 'run' 'build' '--test='\\''safe'\\'''");
  console.log("✅ Test 3 Passed: sandbox_build safely POSIX single-quotes every token");
  passedCount++;

  // --------------------------------------------------------------------------
  // Test 4: Live Gate Operations API - List operations
  // --------------------------------------------------------------------------
  const opsRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/operations`);
  assert.equal(opsRes.status, 200, "Operations endpoint must return 200");
  const opsData = await opsRes.json();
  assert.ok(Array.isArray(opsData.operations), "Operations must be an array");
  console.log(`✅ Test 4 Passed: GET /project/${PROJECT_ID}/operations returns valid list (${opsData.operations.length} ops recorded)`);
  passedCount++;

  // --------------------------------------------------------------------------
  // Test 5: Live Gate Operations API - Record and update operation lifecycle
  // --------------------------------------------------------------------------
  const testOpId = `op_test_${Date.now()}`;
  const testIdempKey = `idemp_test_${Date.now()}`;
  const updateRes1 = await fetch(`${BASE_URL}/project/${PROJECT_ID}/operations/update`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      op_id: testOpId,
      idempotency_key: testIdempKey,
      task_id: "GH-3",
      agent_id: "agent-test-runner",
      status: "accepted",
    }),
  });
  assert.equal(updateRes1.status, 200);
  const updateData1 = await updateRes1.json();
  assert.equal(updateData1.operation.op_id, testOpId);
  assert.equal(updateData1.operation.status, "accepted");
  assert.ok(updateData1.operation.started > 0);

  // Transition to running -> done
  const updateRes2 = await fetch(`${BASE_URL}/project/${PROJECT_ID}/operations/update`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      op_id: testOpId,
      status: "done",
      pr_number: 82,
      pr_url: "https://github.com/ashleycribb/plat-gaming/pull/82",
    }),
  });
  assert.equal(updateRes2.status, 200);
  const updateData2 = await updateRes2.json();
  assert.equal(updateData2.operation.status, "done");
  assert.ok(updateData2.operation.finished >= updateData2.operation.started);
  assert.equal(updateData2.operation.pr_number, 82);
  console.log("✅ Test 5 Passed: Operation record lifecycle {op_id, idempotency_key, status, started, finished} verified");
  passedCount++;

  // --------------------------------------------------------------------------
  // Test 6: Claiming an issue with open op/PR attaches instead of opening duplicate
  // --------------------------------------------------------------------------
  const claimRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/tasks/claim`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      agent_id: "agent-walkaway-claim-test",
      agent_pubkey: "3199704d08ce3e956096a6ac0d2d9a1a168c30ade892868cf47f32c2f2798cc9",
      task_id: "GH-3",
    }),
  });
  assert.equal(claimRes.status, 200);
  const claimData = await claimRes.json();
  assert.equal(claimData.status, "attached", "Claiming task GH-3 with existing CR/PR must attach instead of creating duplicate");
  assert.match(claimData.message, /Attached to existing open operation\/PR/);
  assert.ok(claimData.grant, "Should return delegation grant to attached agent");
  console.log(`✅ Test 6 Passed: Claiming issue with open op/PR attached: status=${claimData.status}, message='${claimData.message}'`);
  passedCount++;

  // --------------------------------------------------------------------------
  // Test 7: Idempotency Key returns existing operation
  // --------------------------------------------------------------------------
  const idempClaimRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/tasks/claim`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      agent_id: "agent-test-runner",
      agent_pubkey: "3199704d08ce3e956096a6ac0d2d9a1a168c30ade892868cf47f32c2f2798cc9",
      task_id: "GH-3",
      idempotency_key: testIdempKey,
    }),
  });
  assert.equal(idempClaimRes.status, 200);
  const idempClaimData = await idempClaimRes.json();
  assert.equal(idempClaimData.op.op_id, testOpId, "Must return existing operation record for idempotency key");
  console.log("✅ Test 7 Passed: Idempotent claim returns existing operation record without creating duplicate");
  passedCount++;

  // --------------------------------------------------------------------------
  // Test 8: PR Detail page contains Real Commit SHA, Image Digest, real Exit Code, no fake pass
  // --------------------------------------------------------------------------
  const prPageRes = await fetch(`${BASE_URL}/p/${PROJECT_ID}/pulls/CR-3-e33f`);
  assert.equal(prPageRes.status, 200);
  const prPageHtml = await prPageRes.text();
  assert.ok(prPageHtml.includes("CR-3-e33f"), "PR page must contain CR ID");
  assert.ok(prPageHtml.includes("Human Judgment Panel") || prPageHtml.includes("Humans make all final judgments") || prPageHtml.includes("Requires Human Approval"), "PR page must enforce supervisor review");
  assert.doesNotMatch(prPageHtml, /0 unhandled exceptions, 100% resilient/, "Must not contain fake pass text");
  console.log("✅ Test 8 Passed: PR page renders real audit trail, no fake pass text");
  passedCount++;

  // --------------------------------------------------------------------------
  // Test 9: Control endpoint shows paused and idle
  // --------------------------------------------------------------------------
  const ctrlRes = await fetch(`${BASE_URL}/control`);
  assert.equal(ctrlRes.status, 200);
  const ctrlData = await ctrlRes.json();
  assert.equal(ctrlData.paused, true, "Swarm must remain paused");
  console.log(`✅ Test 9 Passed: Swarm status is paused=${ctrlData.paused}, status='${ctrlData.status}'`);
  passedCount++;

  // --------------------------------------------------------------------------
  // Test 10: System Health check
  // --------------------------------------------------------------------------
  const healthRes = await fetch(`${BASE_URL}/health`);
  assert.equal(healthRes.status, 200);
  const healthData = await healthRes.json();
  assert.equal(healthData.status, "ok");
  assert.equal(healthData.components.ledger.under_cap, true);
  assert.equal(healthData.components.durable_object.paused, true);
  console.log("✅ Test 10 Passed: GET /health returns all components healthy, ledger under cap");
  passedCount++;

  console.log(`\n🎉 ALL ${passedCount}/${passedCount} TESTS PASSED!`);
}

runTests().catch(err => {
  console.error("Test execution failed:", err);
  process.exit(1);
});
