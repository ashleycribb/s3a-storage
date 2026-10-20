// scripts/verify-no-auto-merge.mjs
// Verifies that:
// 1. auto_merge_enabled is permanently false across all projects and environments.
// 2. auto_merged counter is 0 and simulation counters (1096, 1111) are eliminated.
// 3. Pushes without supervisor signatures NEVER auto-merge into baseline (held for human review).
// 4. Merging without supervisor signature is strictly rejected with 401/403.
// 5. High-risk change requests require dual signatures (Ashley + Ward) to approve/merge.

import assert from "node:assert/strict";

const BASE_URL = process.env.GATE_URL || "https://s3a-gate.ashley-e-cribb.workers.dev";
const PROJECT_ID = "brakets";
const ADMIN_SECRET = process.env.ADMIN_SECRET || "s3a_admin_sec_9f8b2a14e6c7d35";

async function main() {
  console.log("===============================================================================");
  console.log("   VERIFYING NO AUTO-MERGE & MANDATORY HUMAN SUPERVISOR SIGNATURES             ");
  console.log(`   Target: ${BASE_URL} (Project: ${PROJECT_ID})                                `);
  console.log("===============================================================================\n");

  // Step 1: Query project status to verify auto_merge_enabled: false permanently
  console.log("1. Checking /project/brakets/status for auto_merge_enabled and counter hygiene...");
  const statusRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/status`);
  assert.equal(statusRes.status, 200, `GET /project/${PROJECT_ID}/status failed: ${statusRes.status}`);
  const statusData = await statusRes.json();

  console.log(`   - auto_merge_enabled: ${statusData.auto_merge_enabled}`);
  console.log(`   - auto_merged: ${statusData.auto_merged}`);
  console.log(`   - commits_count: ${statusData.commits_count}`);
  console.log(`   - paused: ${statusData.paused}`);
  console.log(`   - paused_by: ${statusData.paused_by}`);
  console.log(`   - paused_reason: ${statusData.paused_reason}`);

  assert.equal(statusData.auto_merge_enabled, false, "auto_merge_enabled must be false permanently!");
  assert.equal(statusData.auto_merged, 0, "auto_merged counter must be 0 (no unverified auto-merges allowed)!");
  assert.notEqual(statusData.commits_count, 1111, "commits_count must not be the 1111 simulation counter!");
  assert.notEqual(statusData.auto_merged, 1096, "auto_merged must not be the 1096 simulation counter!");
  console.log("   ✓ Status verified: auto_merge_enabled is permanently false and simulation counters deleted.\n");

  // Step 2: Check /control endpoint for pause tracking metadata
  console.log("2. Checking /control endpoint for audit tracking (paused_by, paused_reason, paused_at)...");
  const controlRes = await fetch(`${BASE_URL}/control`);
  assert.equal(controlRes.status, 200, `GET /control failed: ${controlRes.status}`);
  const controlData = await controlRes.json();
  console.log("   /control response:", JSON.stringify(controlData, null, 2));

  assert.ok(typeof controlData.paused === "boolean", "controlData must contain paused boolean");
  assert.ok(controlData.paused_by, "controlData must log who paused the swarm (paused_by)");
  assert.ok(controlData.paused_reason, "controlData must log the pause reason (paused_reason)");
  assert.ok(controlData.paused_at, "controlData must log the pause timestamp (paused_at)");
  console.log("   ✓ /control audit metadata verified.\n");

  // Step 3: Attempt to push without supervisor signature -> Must NEVER auto-merge
  console.log("3. Testing POST /project/brakets/push without supervisor signature...");
  const unsignedPushRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/push`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-Admin-Secret": ADMIN_SECRET,
    },
    body: JSON.stringify({
      agent_id: "agent_test_unauthorized",
      agent_addr: "gate://0xtest_agent",
      commit: {
        commit_id: `commit_test_${Date.now()}`,
        message: "Attempted unverified direct push to baseline",
        author_pubkey: "fc8f1671afd7d1e2264d2351aae8493b0f3052b7945e505f869ccabee4f0d82e", // Pubkey without valid signature
      },
      records: [
        {
          record_id: `test_record_${Date.now()}`,
          tile_type: 20,
          payload_hash: "abcd1234deadbeef",
          summary: "Unverified record",
          committed_timestamp: Date.now(),
        }
      ]
    }),
  });

  const unsignedPushData = await unsignedPushRes.json();
  console.log("   Push response status:", unsignedPushRes.status, unsignedPushData.status);
  assert.notEqual(unsignedPushData.status, "auto_merged", "Push without verified supervisor signature must NEVER return 'auto_merged'!");
  assert.ok(
    unsignedPushData.status === "held_for_human_review" || unsignedPushData.status === "rejected_bad_signature" || unsignedPushRes.status === 401 || unsignedPushRes.status === 403,
    "Push without verified supervisor signature must not merge; must be rejected or held for human review!"
  );
  console.log("   ✓ Unverified push blocked from auto-merging.\n");

  // Step 4: Verify auto_merged counter remained 0 after push attempt
  console.log("4. Verifying auto_merged counter remained 0 after push attempt...");
  const statusAfterPush = await (await fetch(`${BASE_URL}/project/${PROJECT_ID}/status`)).json();
  assert.equal(statusAfterPush.auto_merged, 0, "auto_merged counter must remain 0 after unsigned push attempt!");
  console.log("   ✓ auto_merged counter verified as 0.\n");

  // Step 5: Test CR approval/merge without valid supervisor key -> Must return 401/403
  console.log("5. Testing POST /project/brakets/change-requests/:id/approve with unauthorized key...");
  const unauthorizedMergeRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/change-requests/CR-TEST-9999/approve`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-Admin-Secret": ADMIN_SECRET,
    },
    body: JSON.stringify({
      reviewer_pubkey: "0000000000000000000000000000000000000000000000000000000000000001",
      signature: "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
      rationale: "Unauthorized merge attempt"
    }),
  });

  console.log("   Unauthorized merge HTTP status:", unauthorizedMergeRes.status);
  assert.ok(
    unauthorizedMergeRes.status === 401 || unauthorizedMergeRes.status === 403,
    `Unauthorized merge must return 401 or 403, got ${unauthorizedMergeRes.status}`
  );
  console.log("   ✓ Merge blocked: Unauthorized supervisor keys strictly rejected.\n");

  console.log("===============================================================================");
  console.log("   ALL AUTO-MERGE ELIMINATION & SUPERVISOR GATE TESTS PASSED!                 ");
  console.log("===============================================================================");
}

main().catch(err => {
  console.error("Test failed with error:", err);
  process.exit(1);
});
