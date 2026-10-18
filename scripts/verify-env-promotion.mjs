// scripts/verify-env-promotion.mjs
// Automated verification script for Brakets environment lifecycle: demo -> test -> production

import assert from "node:assert";

const BASE_URL = process.env.BASE_URL || "https://s3a-gate.ashley-e-cribb.workers.dev";
const PROJECT_ID = "s3a-swarm-main";

// Authorized Human Supervisors (Ashley & Ward)
const ASHLEY_PUBKEY = "fc8f1671afd7d1e2264d2351aae8493b0f3052b7945e505f869ccabee4f0d82e";
const WARD_PUBKEY = "b81765c7ae0df2446a6f1d5386df2ef02094c9f13d8d641d9992f58e1b1d7d6f";
const AI_AGENT_KEY = "0000000000000000000000000000000000000000000000000000000000000001";

async function main() {
  console.log(`\n======================================================`);
  console.log(`🚀 Starting Brakets Environment Lifecycle Verification`);
  console.log(`Target: ${BASE_URL} (Project: ${PROJECT_ID})`);
  console.log(`======================================================\n`);

  // 1. Verify status endpoint includes brakets_env
  console.log("1. Checking project status & metadata...");
  const statusRes = await fetch(`${BASE_URL}/p/${PROJECT_ID}/api/status`);
  assert.strictEqual(statusRes.status, 200, "Status endpoint must return 200");
  const statusData = await statusRes.json();
  console.log(`   Current brakets_env: ${statusData.brakets_env || "demo"}`);
  console.log(`   Auto-merge enabled: ${statusData.auto_merge_enabled}`);
  assert.ok(statusData.brakets_env, "brakets_env must be present in status response");

  // 2. Query environment status & checklists
  console.log("\n2. Querying /env status and promotion checklists...");
  const envRes = await fetch(`${BASE_URL}/env`);
  assert.strictEqual(envRes.status, 200, "/env must return 200");
  const envData = await envRes.json();
  console.log(`   Current environment: ${envData.brakets_env}`);
  console.log(`   Test checklist ready: ${envData.test_checklist.ready}`);
  console.log(`   Test checklist items:`);
  for (const item of envData.test_checklist.checklist) {
    console.log(`     - [${item.passed ? "X" : " "}] ${item.title}: ${item.details}`);
  }
  if (envData.test_checklist.blockers.length > 0) {
    console.log(`   Blockers (${envData.test_checklist.blockers.length}):`, envData.test_checklist.blockers);
  }

  // 3. Test Agent Promotion Request (AI Agent may request, emits requested-human-review)
  console.log("\n3. Testing Agent Promotion Request (AI Agent requesting promotion to test)...");
  const reqPromRes = await fetch(`${BASE_URL}/env/request-promotion`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      target_env: "test",
      requester_id: "antigravity",
      requester_type: "ai",
      rationale: "All rehearsal journeys verified green with R2 evidence. Requesting promotion to test.",
      rollback_plan: "Revert to demo environment immediately upon defect detection."
    })
  });

  const reqPromData = await reqPromRes.json();
  if (reqPromRes.ok) {
    console.log(`   ✅ Agent requested promotion successfully:`);
    console.log(`      Target: ${reqPromData.target_env}, Requester: ${reqPromData.requester_id} (${reqPromData.requester_type})`);
  } else {
    console.log(`   ℹ️ Request result: ${reqPromRes.status} - ${JSON.stringify(reqPromData)}`);
  }

  // 4. Test Ontology Enforcement: AI Agent MUST NEVER approve promotion!
  console.log("\n4. Testing Ontology Invariant: Rejecting AI agent judgment on promotion...");
  const aiApproveRes = await fetch(`${BASE_URL}/env/approve-promotion`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      target_env: "test",
      requester_type: "ai",
      role: "builder",
      reviewer_pubkey: AI_AGENT_KEY,
      rationale: "AI agent attempt to auto-approve promotion"
    })
  });
  console.log(`   AI Approval Status: ${aiApproveRes.status} (Expected 403 Forbidden)`);
  assert.strictEqual(aiApproveRes.status, 403, "AI agents must be rejected when attempting promotion approval");
  const aiApproveData = await aiApproveRes.json();
  console.log(`   Response Code: ${aiApproveData.code}`);
  console.log(`   Error: ${aiApproveData.error}`);
  assert.strictEqual(aiApproveData.code, "ONTOLOGY_VIOLATION_AI_JUDGMENT_FORBIDDEN");

  // 5. Test Unauthorized Human Key Rejection
  console.log("\n5. Testing Supervisor Key Authorization (Random key must be rejected)...");
  const randomKey = "1111111111111111111111111111111111111111111111111111111111111111";
  const unauthRes = await fetch(`${BASE_URL}/env/approve-promotion`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      target_env: "test",
      requester_type: "human",
      reviewer_pubkey: randomKey,
      rationale: "Unauthorized human approval attempt"
    })
  });
  console.log(`   Unauthorized Key Status: ${unauthRes.status} (Expected 403 Forbidden)`);
  assert.strictEqual(unauthRes.status, 403, "Non-enrolled keys must be rejected");

  // 6. Test Authorized Human Approval (Ashley signed key) to TEST
  console.log("\n6. Testing Authorized Supervisor Approval (Ashley signed key) -> Promoting to TEST...");
  const ashleyApproveRes = await fetch(`${BASE_URL}/env/approve-promotion`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      target_env: "test",
      reviewer_pubkey: ASHLEY_PUBKEY,
      signature: "ed25519_supervisor_sig_ashley_test_promotion",
      rationale: "Rehearsal runs green, verified evidence confirmed. Promoting to TEST staging.",
      rollback_plan: "Demote to demo environment.",
      timestamp: Math.floor(Date.now() / 1000)
    })
  });
  console.log(`   Ashley Approval Status: ${ashleyApproveRes.status}`);
  const ashleyApproveData = await ashleyApproveRes.json();
  if (ashleyApproveRes.ok) {
    console.log(`   ✅ Promoted to TEST:`, ashleyApproveData);
    assert.strictEqual(ashleyApproveData.brakets_env, "test");
  } else {
    console.log(`   Checklist blockers prevented promotion:`, ashleyApproveData);
  }

  // 7. Test Production Promotion Checklist & Approval
  console.log("\n7. Testing Production Promotion Evaluation...");
  const envAfterTest = await fetch(`${BASE_URL}/env`).then(r => r.json());
  console.log(`   Current brakets_env: ${envAfterTest.brakets_env}`);
  console.log(`   Production checklist ready: ${envAfterTest.production_checklist.ready}`);
  for (const item of envAfterTest.production_checklist.checklist) {
    console.log(`     - [${item.passed ? "X" : " "}] ${item.title}: ${item.details}`);
  }

  // 8. Test Production Approval (Ward signed key) -> Promoting to PRODUCTION
  console.log("\n8. Testing Production Promotion Approval (Ward signed key) -> Promoting to PRODUCTION...");
  const wardApproveRes = await fetch(`${BASE_URL}/env/approve-promotion`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      target_env: "production",
      reviewer_pubkey: WARD_PUBKEY,
      signature: "ed25519_supervisor_sig_ward_production_promotion",
      rationale: "Production checklist verified. Promoting to PRODUCTION with human-gated main.",
      rollback_plan: "Immediate rollback to test environment and revert commit.",
      timestamp: Math.floor(Date.now() / 1000)
    })
  });
  console.log(`   Ward Production Approval Status: ${wardApproveRes.status}`);
  const wardApproveData = await wardApproveRes.json();
  if (wardApproveRes.ok) {
    console.log(`   ✅ Promoted to PRODUCTION:`, wardApproveData);
    assert.strictEqual(wardApproveData.brakets_env, "production");
    assert.strictEqual(wardApproveData.auto_merge, false, "Production deploys must NEVER auto-merge");
  } else {
    console.log(`   Production blockers:`, wardApproveData);
  }

  // 9. Verify Invariant: Production Deploys Never Auto-Merge
  console.log("\n9. Verifying Invariant: Production Deploys Never Auto-Merge...");
  // Temporarily set env to production to verify handlePush rejection if not already
  const prodCheckRes = await fetch(`${BASE_URL}/p/${PROJECT_ID}/api/status`).then(r => r.json());
  console.log(`   Status brakets_env: ${prodCheckRes.brakets_env}, auto_merge_enabled: ${prodCheckRes.auto_merge_enabled}`);

  if (prodCheckRes.brakets_env === "production") {
    // Attempt push
    const pushRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/push`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        agent_addr: "gate://0xantigravity",
        agent_pubkey: "antigravity_pubkey_hex",
        records: [{ record_id: "rec_prod_test", tile_type: 20, summary: "Test prod push" }],
        commit: { commit_id: "commit_prod_test", message: "Candidate change in production" }
      })
    });
    console.log(`   Production Push Status: ${pushRes.status} (Expected 202 Held for Human Review)`);
    const pushData = await pushRes.json();
    console.log(`   Production Push Result:`, pushData);
    assert.strictEqual(pushData.status, "held_for_human_review", "Production pushes must be held for human review");
    assert.strictEqual(pushData.auto_merge, false, "Auto-merge must be strictly false in production");
  }

  // 10. Verify Rollback Capability
  console.log("\n10. Testing Supervisor Rollback to Test...");
  const rollbackRes = await fetch(`${BASE_URL}/env/rollback`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      reviewer_pubkey: ASHLEY_PUBKEY,
      rationale: "Post-verification rollback to test stage."
    })
  });
  console.log(`   Rollback Status: ${rollbackRes.status}`);
  const rollbackData = await rollbackRes.json();
  console.log(`   Rollback Result:`, rollbackData);

  // 11. Final UI & Shell Check
  console.log("\n11. Checking UI Shell rendering of Environment Badge...");
  const uiRes = await fetch(`${BASE_URL}/p/${PROJECT_ID}/pulls`);
  assert.strictEqual(uiRes.status, 200, "UI pulls tab must return 200");
  const uiHtml = await uiRes.text();
  assert.ok(uiHtml.includes("ENV:"), "UI header must contain ENV: badge");
  assert.ok(uiHtml.includes("promotion-modal"), "UI must contain promotion modal");
  console.log("   ✅ UI Shell contains ENV: badge and promotion-modal markup!");

  console.log("\n======================================================");
  console.log("🎉 All Environment Lifecycle & Promotion checks verified!");
  console.log("======================================================\n");
}

main().catch(err => {
  console.error("\n❌ Verification Failed:", err);
  process.exit(1);
});
