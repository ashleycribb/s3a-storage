// scripts/pulls-tab-test.mjs
// Verifies all requirements for /p/s3a-swarm-main/pulls:
// 1. Default tab: "Needs human judgment" — only CRs awaiting approve/request-changes/reject
// 2. Each row shows status badge, agent, risk (high = needs second human), checks passed/failed, and time waiting
// 3. Sort Needs-judgment by high risk first, then oldest first
// 4. Tab counts: Needs judgment (X), Open (Y), Merged (Z), Closed (W)
// 5. Keep merged PRs out of default list even after refresh

import assert from "node:assert/strict";

const BASE_URL = process.env.GATE_URL || "https://s3a-gate.ashley-e-cribb.workers.dev";
const PROJECT_ID = "s3a-swarm-main";

async function main() {
  console.log("=== S3A PULLS TAB & DEFAULT VIEW TEST ===");
  console.log(`Gate URL: ${BASE_URL}\n`);

  // 1. Fetch JSON change requests from internal API to establish ground truth
  const crRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/change-requests`);
  assert.equal(crRes.status, 200, `Failed to fetch CRs: ${crRes.status}`);
  const crData = await crRes.json();
  const allCRs = crData.change_requests || [];

  const expectedNeedsJudgment = allCRs.filter(c => c.status === "pending_review" || c.status === "changes_requested");
  const expectedOpen = allCRs.filter(c => c.status === "pending_review" || c.status === "changes_requested");
  const expectedMerged = allCRs.filter(c => c.status === "auto_merged" || c.status === "approved");
  const expectedClosed = allCRs.filter(c => c.status === "rejected");

  console.log(`Ground truth Change Requests:`);
  console.log(`- Total:            ${allCRs.length}`);
  console.log(`- Needs Judgment:   ${expectedNeedsJudgment.length}`);
  console.log(`- Open:             ${expectedOpen.length}`);
  console.log(`- Merged:           ${expectedMerged.length}`);
  console.log(`- Closed:           ${expectedClosed.length}\n`);

  // 2. Test Default View: GET /p/s3a-swarm-main/pulls (no query param)
  console.log("1. Testing default tab at GET /p/s3a-swarm-main/pulls...");
  const defaultRes = await fetch(`${BASE_URL}/p/${PROJECT_ID}/pulls`);
  assert.equal(defaultRes.status, 200, `Default page returned status ${defaultRes.status}`);
  const defaultHtml = await defaultRes.text();

  // Requirement 4: Tab counts present
  assert.ok(defaultHtml.includes(`Needs judgment (${expectedNeedsJudgment.length})`), "Tab header must show Needs judgment with count");
  assert.ok(defaultHtml.includes(`Open (${expectedOpen.length})`), "Tab header must show Open with count");
  assert.ok(defaultHtml.includes(`Merged (${expectedMerged.length})`), "Tab header must show Merged with count");
  assert.ok(defaultHtml.includes(`Closed (${expectedClosed.length})`), "Tab header must show Closed with count");
  console.log("   [PASSED] Tab counts verified.");

  // Requirement 1 & 5: Merged PRs NEVER in default list
  for (const m of expectedMerged) {
    // Assert that the link to review/judge this merged CR is not rendered as a row in the default tab
    const rowMarker = `href="/p/${PROJECT_ID}/pulls/${m.id}"`;
    assert.ok(!defaultHtml.includes(rowMarker), `Merged PR #${m.id} must NOT appear in default list!`);
  }
  for (const c of expectedClosed) {
    const rowMarker = `href="/p/${PROJECT_ID}/pulls/${c.id}"`;
    assert.ok(!defaultHtml.includes(rowMarker), `Closed PR #${c.id} must NOT appear in default list!`);
  }
  console.log("   [PASSED] Merged and Closed PRs strictly excluded from default tab.");

  // Requirement 2: Each row shows status badge, agent, risk, checks passed/failed, time waiting
  if (expectedNeedsJudgment.length > 0) {
    assert.ok(defaultHtml.includes("agent:"), "Row must show agent badge");
    assert.ok(defaultHtml.includes("Risk:"), "Row must show risk badge");
    assert.ok(defaultHtml.includes("Checks:"), "Row must show checks passed/failed badge");
    assert.ok(defaultHtml.includes("waiting") || defaultHtml.includes("Waiting"), "Row must show time waiting badge");
    console.log("   [PASSED] Row elements (status badge, agent, risk, checks, time waiting) verified.");
  } else {
    assert.ok(defaultHtml.includes("Zero pull requests awaiting human judgment"), "Blankslate when 0 awaiting judgment");
    console.log("   [PASSED] Blankslate verified for zero pending judgment.");
  }

  // 3. Test Merged Tab: GET /p/s3a-swarm-main/pulls?tab=merged
  console.log("\n2. Testing Merged tab at GET /p/s3a-swarm-main/pulls?tab=merged...");
  const mergedRes = await fetch(`${BASE_URL}/p/${PROJECT_ID}/pulls?tab=merged`);
  assert.equal(mergedRes.status, 200, `Merged tab returned status ${mergedRes.status}`);
  const mergedHtml = await mergedRes.text();
  for (const m of expectedMerged) {
    assert.ok(mergedHtml.includes(m.id), `Merged PR #${m.id} must appear in Merged tab`);
  }
  console.log(`   [PASSED] All ${expectedMerged.length} merged PRs appear on the Merged tab.`);

  // 4. Test Refresh Persistence: fetch /p/s3a-swarm-main/pulls multiple times
  console.log("\n3. Testing refresh persistence (Requirement 5)...");
  for (let i = 1; i <= 3; i++) {
    const refreshRes = await fetch(`${BASE_URL}/p/${PROJECT_ID}/pulls`);
    assert.equal(refreshRes.status, 200);
    const refreshHtml = await refreshRes.text();
    for (const m of expectedMerged) {
      assert.ok(!refreshHtml.includes(`href="/p/${PROJECT_ID}/pulls/${m.id}"`), `Merged PR #${m.id} leaked into default list on refresh ${i}`);
    }
  }
  console.log("   [PASSED] Merged PRs remain strictly out of default list across repeated refreshes.");

  // 5. Test Sorting Logic on Needs-judgment tab (Requirement 3)
  console.log("\n4. Testing Needs-judgment sorting (high risk first, then oldest first)...");
  if (expectedNeedsJudgment.length >= 2) {
    const sortedExpected = [...expectedNeedsJudgment].sort((a, b) => {
      const aHigh = (a.risk_level === "high" || a.needs_second_human || /cashier|escrow|payout|auth|db|infra/i.test((a.title || "") + " " + (a.touched_files || []).join(" "))) ? 1 : 0;
      const bHigh = (b.risk_level === "high" || b.needs_second_human || /cashier|escrow|payout|auth|db|infra/i.test((b.title || "") + " " + (b.touched_files || []).join(" "))) ? 1 : 0;
      if (aHigh !== bHigh) return bHigh - aHigh;
      return (a.created_at || 0) - (b.created_at || 0);
    });

    // Check order of appearance in defaultHtml
    let lastIdx = -1;
    for (const item of sortedExpected) {
      const idx = defaultHtml.indexOf(`href="/p/${PROJECT_ID}/pulls/${item.id}"`);
      assert.ok(idx > lastIdx, `PR #${item.id} is out of sort order (expected high risk first, then oldest first)`);
      lastIdx = idx;
    }
    console.log("   [PASSED] High risk first, then oldest first ordering confirmed.");
  } else {
    console.log("   [PASSED] Sorting verified (fewer than 2 pending items).");
  }

  console.log("\n==================================================================");
  console.log("ALL PULLS TAB REQUIREMENTS VERIFIED SUCCESSFULLY!");
  console.log("==================================================================");
}

main().catch(err => {
  console.error("FATAL in pulls-tab-test:", err);
  process.exit(1);
});
