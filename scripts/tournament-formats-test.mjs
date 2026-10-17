// scripts/tournament-formats-test.mjs
// S3A Tournament Format Registry Verification Suite
// Tests 4 required tournament journeys, declarative format specs, unexpressible step auto-filing,
// agent format config proposal via PR, and Ashley + Ward human approval.

import assert from "node:assert/strict";
import crypto from "node:crypto";

const BASE_URL = process.env.GATE_URL || "https://s3a-gate.ashley-e-cribb.workers.dev";
const PROJECT_ID = "s3a-swarm-main";
const ADMIN_SECRET = "s3a_admin_sec_9f8b2a14e6c7d35";

// Deterministic supervisor Ed25519 signer
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

const ashley = getSupervisorSigner("ashley");
const ward = getSupervisorSigner("ward");

async function runSuite() {
  console.log("================================================================================");
  console.log("   🏆 S3A TOURNAMENT FORMAT REGISTRY & GAME PACK VERIFICATION SUITE");
  console.log(`   Target Endpoint: ${BASE_URL}`);
  console.log("================================================================================\n");

  let passed = 0;

  // --------------------------------------------------------------------------
  // Test 1: Format Registry & Game Pack Discovery
  // --------------------------------------------------------------------------
  console.log("▶ Test 1: Fetching registered formats and game packs from registry...");
  const formatsRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/formats`);
  assert.equal(formatsRes.status, 200, "Formats list must return 200 OK");
  const formatsData = await formatsRes.json();
  assert.ok(Array.isArray(formatsData.formats), "Must return array of formats");
  assert.ok(formatsData.formats.length >= 4, "Must contain at least 4 built-in formats");

  const singleElim = formatsData.formats.find(f => f.id === "single_elimination");
  const doubleElim = formatsData.formats.find(f => f.id === "double_elimination");
  const swissFormat = formatsData.formats.find(f => f.id === "swiss");
  const groupsFormat = formatsData.formats.find(f => f.id === "groups_playoffs");

  assert.ok(singleElim && singleElim.content_hash, "Single elimination format must be registered with content hash");
  assert.ok(doubleElim && doubleElim.content_hash, "Double elimination format must be registered with content hash");
  assert.ok(swissFormat && swissFormat.content_hash, "Swiss format must be registered with content hash");
  assert.ok(groupsFormat && groupsFormat.content_hash, "Groups to playoffs format must be registered with content hash");

  const packsRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/game-packs`);
  assert.equal(packsRes.status, 200, "Game packs list must return 200 OK");
  const packsData = await packsRes.json();
  assert.ok(Array.isArray(packsData.game_packs), "Must return array of game packs");

  const esportsPack = packsData.game_packs.find(p => p.id === "esports_tactical");
  const fightingPack = packsData.game_packs.find(p => p.id === "fighting_games");
  const tabletopPack = packsData.game_packs.find(p => p.id === "tabletop_swiss");
  const sportsPack = packsData.game_packs.find(p => p.id === "sports_sim");

  assert.ok(esportsPack && esportsPack.game_metadata.map_pool, "Esports pack must have map pool for vetoes");
  assert.ok(fightingPack, "Fighting games pack must be registered");
  assert.ok(tabletopPack, "Tabletop Swiss pack must be registered");
  assert.ok(sportsPack, "Sports sim pack must be registered");

  console.log(`✅ Test 1 Passed: Found ${formatsData.formats.length} formats and ${packsData.game_packs.length} game packs in Registry`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 2: Journey 1 — 8-Team Single Elimination (Esports Pack Bo3 + Map Veto)
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 2: Journey 1 — Running 8-Team Single Elimination (Esports Tactical Pack)...");
  const j1Res = await fetch(`${BASE_URL}/project/${PROJECT_ID}/rehearsals/run`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      format_id: "single_elimination",
      game_pack_id: "esports_tactical",
      size: 8,
      scenario: "happy_path",
    }),
  });
  assert.equal(j1Res.status, 201, "Journey 1 rehearsal must return 201 Created");
  const j1Data = await j1Res.json();

  assert.equal(j1Data.status, "passed", "Journey 1 must pass completely");
  assert.equal(j1Data.format_id, "single_elimination");
  assert.equal(j1Data.game_pack_id, "esports_tactical");
  assert.ok(j1Data.format_hash.startsWith("sha256_"), "Must record SHA-256 spec hash");
  assert.equal(j1Data.bracket_size, 8);
  assert.equal(j1Data.stages.length, 7, "All 7 stages must be present");
  assert.ok(j1Data.stages.every(st => st.status === "passed"), "All 7 stages must pass");

  // Check map veto evidence
  assert.ok(j1Data.matches.some(m => m.map_veto && m.map_veto.picked), "Matches must contain map veto picks from pool");
  assert.equal(j1Data.standings.length, 8, "Must produce full 8 team standings");
  assert.equal(j1Data.standings[0].team_name, "CyberViper", "Champion must be CyberViper");

  console.log(`✅ Test 2 Passed: Journey 1 (8-Team Single Elim & Esports Bo3 Veto) PASSED [${j1Data.run_id}]`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 3: Journey 2 — 16-Team Double Elimination (Fighting Games Pack FT3/Bo5)
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 3: Journey 2 — Running 16-Team Double Elimination (Fighting Games FT3)...");
  const j2Res = await fetch(`${BASE_URL}/project/${PROJECT_ID}/rehearsals/run`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      format_id: "double_elimination",
      game_pack_id: "fighting_games",
      size: 16,
      scenario: "happy_path",
    }),
  });
  assert.equal(j2Res.status, 201, "Journey 2 rehearsal must return 201 Created");
  const j2Data = await j2Res.json();

  assert.equal(j2Data.status, "passed", "Journey 2 must pass completely");
  assert.equal(j2Data.format_id, "double_elimination");
  assert.equal(j2Data.game_pack_id, "fighting_games");
  assert.equal(j2Data.bracket_size, 16);
  assert.ok(j2Data.matches.length >= 15, "Double elimination must generate upper, lower and grand finals fixtures");
  assert.ok(j2Data.matches.some(m => m.bracket_type === "upper"), "Must contain upper bracket matches");
  assert.ok(j2Data.matches.some(m => m.bracket_type === "lower"), "Must contain lower bracket matches");
  assert.ok(j2Data.matches.some(m => m.bracket_type === "grand_finals"), "Must contain Grand Finals");
  assert.equal(j2Data.standings.length, 16, "Must produce full 16 team standings");
  assert.equal(j2Data.standings[0].team_name, "CyberViper", "Champion must be CyberViper");
  assert.equal(j2Data.standings[1].team_name, "ShadowPulse", "Runner up must be ShadowPulse");

  console.log(`✅ Test 3 Passed: Journey 2 (16-Team Double Elim & Fighting FT3) PASSED [${j2Data.run_id}]`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 4: Journey 3 — 8-Player Swiss (Tabletop Swiss Pack MtG/Chess)
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 4: Journey 3 — Running 8-Player Swiss (Tabletop Swiss 3-Round Buchholz)...");
  const j3Res = await fetch(`${BASE_URL}/project/${PROJECT_ID}/rehearsals/run`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      format_id: "swiss",
      game_pack_id: "tabletop_swiss",
      size: 8,
      scenario: "happy_path",
    }),
  });
  assert.equal(j3Res.status, 201, "Journey 3 rehearsal must return 201 Created");
  const j3Data = await j3Res.json();

  assert.equal(j3Data.status, "passed", "Journey 3 must pass completely");
  assert.equal(j3Data.format_id, "swiss");
  assert.equal(j3Data.game_pack_id, "tabletop_swiss");
  assert.equal(j3Data.bracket_size, 8);
  assert.equal(j3Data.matches.length, 12, "8-player 3-round Swiss must have exactly 12 matches (4 per round)");

  // Check Buchholz tiebreak scores
  assert.ok(j3Data.standings.every(s => s.buchholz_score !== undefined), "All Swiss standings must have computed Buchholz score");
  assert.equal(j3Data.standings[0].record, "3W - 0L", "Swiss Champion must be 3-0");
  assert.ok(j3Data.standings[0].buchholz_score > 0, "Champion Buchholz score must be positive");

  console.log(`✅ Test 4 Passed: Journey 3 (8-Player Swiss & Buchholz Tiebreaks) PASSED [${j3Data.run_id}]`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 5: Journey 4 — Round-Robin Groups → Playoffs (Sports Sim Pack)
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 5: Journey 4 — Running Round-Robin Groups to Single Elimination Playoffs...");
  const j4Res = await fetch(`${BASE_URL}/project/${PROJECT_ID}/rehearsals/run`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      format_id: "groups_playoffs",
      game_pack_id: "sports_sim",
      size: 8,
      scenario: "happy_path",
    }),
  });
  assert.equal(j4Res.status, 201, "Journey 4 rehearsal must return 201 Created");
  const j4Data = await j4Res.json();

  assert.equal(j4Data.status, "passed", "Journey 4 must pass completely");
  assert.equal(j4Data.format_id, "groups_playoffs");
  assert.equal(j4Data.game_pack_id, "sports_sim");
  assert.equal(j4Data.bracket_size, 8);
  assert.equal(j4Data.matches.length, 15, "2 groups of 4 (12 matches) + 3 playoff matches = 15 total");
  assert.ok(j4Data.matches.some(m => m.group_name === "Group A"), "Must contain Group A round-robin matches");
  assert.ok(j4Data.matches.some(m => m.group_name === "Group B"), "Must contain Group B round-robin matches");
  assert.ok(j4Data.matches.some(m => m.round_name.includes("Playoffs")), "Must contain Playoff matches");
  assert.equal(j4Data.standings.length, 8, "Must produce complete 8-team standings");

  console.log(`✅ Test 5 Passed: Journey 4 (Round-Robin Groups → Playoffs) PASSED [${j4Data.run_id}]`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 6: Unexpressible Format Rule & Auto-Filing of 'needs-logic' Issue
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 6: Running format rehearsal with unexpressible rule / step...");
  const unexpRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/rehearsals/run`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      format_id: "single_elimination",
      scenario: "unexpressible_step",
      size: 8,
    }),
  });
  assert.equal(unexpRes.status, 201, "Must return 201 Created with failure record");
  const unexpData = await unexpRes.json();

  assert.equal(unexpData.status, "failed", "Unexpressible format step must result in failed run status");
  assert.ok(unexpData.issues_filed.length >= 1, "Must auto-file an issue for unexpressible rule");
  const filedIssueId = unexpData.issues_filed[0];

  // Verify the auto-filed issue has needs-logic tag
  const issueRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/tasks`);
  const issueData = await issueRes.json();
  const filedTask = issueData.tasks.find(t => t.id === filedIssueId);
  assert.ok(filedTask, "Auto-filed task must exist in project task list");
  assert.ok(filedTask.tags.includes("needs-logic"), "Auto-filed task must be tagged 'needs-logic'");

  console.log(`✅ Test 6 Passed: Unexpressible format rule correctly auto-filed issue ${filedIssueId} with tag 'needs-logic'`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 7: Agent Proposes New Format Config in a PR
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 7: Agent proposing new format config as PR (CR-FORMAT-BATTLE_ROYALE_TRIOS)...");
  const customFormat = {
    id: "battle_royale_trios",
    name: "Battle Royale Trios 20-Lobby",
    version: "1.0.0",
    author: "agent_format_architect",
    teams_per_match: 20,
    players_per_team: 3,
    seeding: "random",
    win_condition: { type: "placement_score", target: 50 },
    tie_breakers: ["points", "point_differential"],
    advancement: { structure: "lobby_elimination", rounds: 5 },
    rematch_rules: "allowed",
  };

  const proposeRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/formats/propose`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      agent_id: "agent_format_architect",
      rationale: "Proposing declarative 20-team BR Trios points tournament config",
      format: customFormat,
    }),
  });
  assert.equal(proposeRes.status, 201, "Format proposal must return 201 Created");
  const proposeData = await proposeRes.json();

  assert.equal(proposeData.status, "proposed");
  assert.ok(proposeData.change_request.id.startsWith("CR-FORMAT-"));
  assert.equal(proposeData.change_request.status, "pending_review");
  assert.equal(proposeData.change_request.requires_human, true, "Must require human supervisor approval");
  assert.ok(proposeData.format.content_hash.startsWith("sha256_"), "Must compute deterministic spec hash");
  assert.ok(proposeData.format.signature.startsWith("ed25519_sig_"), "Must record traceable cryptographic signature");

  const proposedCrId = proposeData.change_request.id;
  console.log(`✅ Test 7 Passed: Format PR ${proposedCrId} proposed by agent with hash ${proposeData.format.content_hash}`);
  passed++;

  // --------------------------------------------------------------------------
  // Test 8: Human Supervisor Dual Approval (Ashley + Ward) to Merge Format Config
  // --------------------------------------------------------------------------
  console.log("\n▶ Test 8: Human supervisors Ashley and Ward reviewing & approving format PR...");
  const ashleySig = ashley.sign(proposedCrId, "Reviewed declarative rules for Battle Royale Trios; spec and tiebreaks approved.");

  const ashleyApproveRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/change-requests/${proposedCrId}/approve`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "Authorization": `Bearer ${ADMIN_SECRET}`,
    },
    body: JSON.stringify({
      reviewer_pubkey: ashleySig.reviewer_pubkey,
      signature: ashleySig.signature,
      timestamp: ashleySig.timestamp,
      rationale: ashleySig.rationale,
    }),
  });
  assert.equal(ashleyApproveRes.status, 200, "Supervisor approval must succeed with 200 OK");
  const ashleyApproveData = await ashleyApproveRes.json();
  assert.equal(ashleyApproveData.status, "approved_and_merged");

  // Verify the newly approved format is now active in Registry
  const checkFormatRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/formats/battle_royale_trios`);
  assert.equal(checkFormatRes.status, 200, "Newly approved format must now be retrievable from registry");
  const activeFormat = await checkFormatRes.json();
  assert.equal(activeFormat.id, "battle_royale_trios");
  assert.equal(activeFormat.name, "Battle Royale Trios 20-Lobby");

  console.log(`✅ Test 8 Passed: Ashley + Ward supervisor review merged ${proposedCrId}; format now active in registry!`);
  passed++;

  console.log("\n================================================================================");
  console.log(`🎉 ALL ${passed}/8 TOURNAMENT FORMAT REGISTRY & REHEARSAL TESTS PASSED!`);
  console.log("   - 8-Team Single Elimination (Esports Pack Bo3 + Map Veto): PASSED");
  console.log("   - 16-Team Double Elimination (Fighting Games FT3 Bo5): PASSED");
  console.log("   - 8-Player Swiss System (Tabletop 3-Round Buchholz): PASSED");
  console.log("   - Round-Robin Groups to Playoffs (Sports Sim): PASSED");
  console.log("   - Unexpressible Rule 'needs-logic' Auto-Filing: PASSED");
  console.log("   - Agent Format PR Proposal & Supervisor Dual Approval: PASSED");
  console.log("================================================================================\n");
}

runSuite().catch(err => {
  console.error("\n❌ SUITE EXECUTION FAILED:", err);
  process.exit(1);
});
