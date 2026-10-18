// scripts/verify-subagent-coordination.mjs
// Automated verification suite for Dynamic Helper Agent Spawning & Coordination:
// 1. PM / Builder decomposes large tasks / code repairs into specialized helper agents
// 2. Mandatory coordination contract: specific_task, goal, bounded files_allowed, and tests_required
// 3. xAPI provenance: trace entry recorded with verb 'assigned'
// 4. GET /project/:id/agents/coordination returns coordination plan and helper agents
// 5. /platform/agents renders coordinated helper agents with task & goal callouts
// 6. Visual Review Canvas renders coordinated helper agents and repair goals

import assert from "node:assert/strict";

const BASE_URL = process.env.GATE_URL || "https://s3a-gate.ashley-e-cribb.workers.dev";
const PROJECT_ID = "s3a-swarm-main";

async function run() {
  console.log("===============================================================================");
  console.log("   VERIFYING DYNAMIC HELPER AGENT SPAWNING & TASK COORDINATION                ");
  console.log(`   Target: ${BASE_URL} (Project: ${PROJECT_ID})                                `);
  console.log("===============================================================================\n");

  // --------------------------------------------------------------------------
  // 1. Trigger Dynamic Agent Coordination for Large Code Repair (TASK-283)
  // --------------------------------------------------------------------------
  console.log("1. Triggering POST /project/" + PROJECT_ID + "/agents/coordinate (TASK-283 repair)...");
  const coordRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/agents/coordinate`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      parent_agent_id: "gate-agent",
      parent_agent_role: "builder",
      task_id: "TASK-283",
      scope_level: "large",
      rationale: "TASK-283 code review remediation spans 5 distinct subsystem finding areas requiring specialized helper agents."
    })
  });

  assert.equal(coordRes.status, 201, `Expected 201 Created from coordinate endpoint, got ${coordRes.status}`);
  const coordData = await coordRes.json();
  assert.ok(coordData.success, "Coordination response must indicate success");
  assert.ok(coordData.coordination_plan, "Must return coordination_plan");
  assert.equal(coordData.coordination_plan.parent_agent_id, "gate-agent");
  assert.equal(coordData.coordination_plan.parent_agent_role, "builder");
  assert.equal(coordData.coordination_plan.task_id, undefined); // plan has spawned_agents
  assert.ok(Array.isArray(coordData.coordination_plan.spawned_agents), "Must contain spawned_agents");
  assert.equal(coordData.coordination_plan.spawned_agents.length, 5, "Must spawn 5 specialized helper agents for TASK-283");

  console.log(`   ✓ gate-agent spawned ${coordData.coordination_plan.spawned_agents.length} helper agents.`);

  // --------------------------------------------------------------------------
  // 2. Verify Coordination Contract (Specific Task & Clear Goal on Every Helper)
  // --------------------------------------------------------------------------
  console.log("\n2. Verifying Coordination Contract on all spawned helper agents...");
  for (const agent of coordData.coordination_plan.spawned_agents) {
    assert.ok(agent.id, "Agent must have an ID");
    assert.ok(agent.name, "Agent must have a name");
    assert.ok(agent.specific_task && agent.specific_task.length > 10, `Agent ${agent.id} must have a detailed specific_task`);
    assert.ok(agent.goal && agent.goal.length > 10, `Agent ${agent.id} must have a clear goal`);
    assert.ok(Array.isArray(agent.files_allowed) && agent.files_allowed.length > 0, `Agent ${agent.id} must have bounded files_allowed`);
    assert.ok(agent.tests_required, `Agent ${agent.id} must have tests_required`);
    assert.equal(agent.parent_agent_id, "gate-agent", "Parent must be gate-agent");
    assert.equal(agent.xapi_provenance, "https://ashecribb.dev/xapi/verbs/assigned", "Must use xapi:assigned");

    console.log(`   - Helper [${agent.id}] ${agent.name}`);
    console.log(`     🎯 Task: ${agent.specific_task}`);
    console.log(`     🏁 Goal: ${agent.goal}`);
    console.log(`     📁 Scope: ${agent.files_allowed.join(', ')}`);
  }
  console.log("   ✓ All helper agents satisfy the coordination contract.");

  // --------------------------------------------------------------------------
  // 3. Query GET /project/:id/agents/coordination
  // --------------------------------------------------------------------------
  console.log("\n3. Querying GET /project/" + PROJECT_ID + "/agents/coordination...");
  const getCoordRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/agents/coordination`);
  assert.equal(getCoordRes.status, 200, `Expected 200 OK from coordination endpoint, got ${getCoordRes.status}`);
  const getCoordData = await getCoordRes.json();
  assert.ok(getCoordData.plan, "Must return active coordination plan");
  assert.equal(getCoordData.spawned_agents.length, 5, "Must return 5 spawned agents");
  console.log(`   ✓ Retrieved active coordination plan (${getCoordData.plan.id}) with ${getCoordData.spawned_agents.length} agents.`);

  // --------------------------------------------------------------------------
  // 4. Query GET /project/:id/agents (Registered Agents List)
  // --------------------------------------------------------------------------
  console.log("\n4. Verifying helper agents registered in Swarm Agents Registry...");
  const agentsRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/agents`);
  assert.equal(agentsRes.status, 200, `Expected 200 OK from agents endpoint, got ${agentsRes.status}`);
  const agentsData = await agentsRes.json();
  const registeredIds = Object.keys(agentsData.agents);
  for (const helper of coordData.coordination_plan.spawned_agents) {
    assert.ok(registeredIds.includes(helper.id), `Helper agent ${helper.id} must be registered in DO storage`);
    const reg = agentsData.agents[helper.id];
    assert.equal(reg.parent_agent_id, "gate-agent");
    assert.equal(reg.specific_task, helper.specific_task);
    assert.equal(reg.goal, helper.goal);
    assert.equal(reg.is_spawned_helper, true);
  }
  console.log(`   ✓ All ${coordData.coordination_plan.spawned_agents.length} helper agents registered with parent link, specific task, and goal.`);

  // --------------------------------------------------------------------------
  // 5. Test Custom Subagent Spawning by PM Agent (Project Refinement)
  // --------------------------------------------------------------------------
  console.log("\n5. Testing PM Agent custom subagent spawning for project refinement...");
  const pmCoordRes = await fetch(`${BASE_URL}/project/${PROJECT_ID}/agents/coordinate`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      parent_agent_id: "pm",
      parent_agent_role: "pm",
      task_id: "TASK-REFINEMENT-401",
      scope_level: "medium",
      rationale: "PM coordinating micro-SaaS Stripe webhook hardening and schema migration.",
      subagents: [
        {
          id: "subagent-stripe-sec",
          name: "Stripe Webhook Security Specialist",
          role: "builder",
          specific_task: "Implement Stripe signature replay protection using timestamp drift tolerance and idempotency key caching",
          goal: "Ensure zero duplicate subscription events can be replayed against webhook endpoints",
          files_allowed: ["app/api/webhooks/stripe/route.ts", "lib/stripe.ts"],
          tests_required: "npm test tests/stripe-webhook.test.ts"
        },
        {
          id: "subagent-d1-migrator",
          name: "D1 Database Schema Specialist",
          role: "builder",
          specific_task: "Draft forward-compatible SQL migrations for tenant subscription limits",
          goal: "Execute zero-downtime D1 migrations without locking tenant tables",
          files_allowed: ["lib/db/schema.ts", "migrations/0002_tenant_limits.sql"],
          tests_required: "npx wrangler d1 migrations apply DB --local"
        }
      ]
    })
  });

  assert.equal(pmCoordRes.status, 201, `Expected 201 Created from PM coordination, got ${pmCoordRes.status}`);
  const pmCoordData = await pmCoordRes.json();
  assert.equal(pmCoordData.coordination_plan.parent_agent_id, "pm");
  assert.equal(pmCoordData.coordination_plan.spawned_agents.length, 2);
  assert.equal(pmCoordData.coordination_plan.spawned_agents[0].specific_task, "Implement Stripe signature replay protection using timestamp drift tolerance and idempotency key caching");
  console.log("   ✓ PM Agent coordinated 2 custom helper agents with explicit tasks and goals.");

  // --------------------------------------------------------------------------
  // 6. Verify Visual Review Canvas & Platform UI Rendering
  // --------------------------------------------------------------------------
  console.log("\n6. Verifying Visual Review Canvas & Platform UI rendering of helper agents...");
  const svgRes = await fetch(`${BASE_URL}/project/brakets/visual-review.svg`);
  assert.equal(svgRes.status, 200, "Visual review SVG must return 200");
  const svgText = await svgRes.text();
  assert.ok(svgText.includes("Coordinated Helper Agents &amp; Tasks"), "SVG canvas must display coordinated helper agents header");
  assert.ok(svgText.includes("Agent: helper-sec-guard"), "SVG canvas must display helper-sec-guard");
  assert.ok(svgText.includes("Goal: Ed25519 2-human multisig required"), "SVG canvas must display security helper goal");
  assert.ok(svgText.includes("Agent: helper-tournament-core"), "SVG canvas must display helper-tournament-core");
  assert.ok(svgText.includes("Goal: Format-aware draw rejection"), "SVG canvas must display tournament core goal");
  console.log("   ✓ Visual Review Canvas renders coordinated helper agents with tasks and goals.");

  const platformAgentsRes = await fetch(`${BASE_URL}/platform/agents`);
  assert.equal(platformAgentsRes.status, 200, "Platform agents page must return 200");
  const platformAgentsHtml = await platformAgentsRes.text();
  assert.ok(platformAgentsHtml.includes("Parent Coordinator:"), "Platform UI must show Parent Coordinator");
  assert.ok(platformAgentsHtml.includes("Specific Task:"), "Platform UI must show Specific Task");
  assert.ok(platformAgentsHtml.includes("Goal:"), "Platform UI must show Goal");
  console.log("   ✓ Platform UI (/platform/agents) renders coordinated helper agents with task & goal callouts.");

  console.log("\n===============================================================================");
  console.log("   ALL DYNAMIC HELPER AGENT SPAWNING & COORDINATION CHECKS PASSED!            ");
  console.log("===============================================================================");
}

run().catch((err) => {
  console.error("\n❌ Verification Failed:", err);
  process.exit(1);
});
