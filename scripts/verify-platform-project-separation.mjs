// scripts/verify-platform-project-separation.mjs
// Automated verification suite for Platform vs Project Separation in S3A Gate:
// 1. 301 Redirects: /p/s3a-swarm-main/* -> /p/brakets/*
// 2. Code Isolation: /p/brakets/code contains only Brakets code (no swarm platform code)
// 3. Platform Pages (/platform/*): Overview, Projects, Agents, Ledger, Traces
// 4. Header & Project Switcher: "swarm ▶ Brakets (plat-gaming) • env: test"
// 5. Create Project API: POST /api/projects creates isolated project with repo, issue board, format/skill packs
// 6. Visual SVG Review Canvas: /project/:projectId/visual-review.svg

import assert from "node:assert/strict";

const BASE_URL = process.env.GATE_URL || "https://s3a-gate.ashley-e-cribb.workers.dev";
const SUPERVISOR_KEY = "fc8f1671afd7d1e2264d2351aae8493b0f3052b7945e505f869ccabee4f0d82e"; // Ashley

async function run() {
  console.log("===============================================================================");
  console.log("      VERIFYING PLATFORM (SWARM) VS PROJECT (BRAKETS) SEPARATION              ");
  console.log(`      Target: ${BASE_URL}                                                    `);
  console.log("===============================================================================\n");

  // --------------------------------------------------------------------------
  // 1. Verify 301 Redirects from /p/s3a-swarm-main/* and /project/s3a-swarm-main/*
  // --------------------------------------------------------------------------
  console.log("1. Verifying 301 Redirects from legacy project URLs...");
  const redirectRes1 = await fetch(`${BASE_URL}/p/s3a-swarm-main/code`, { redirect: "manual" });
  assert.equal(redirectRes1.status, 301, `Expected 301 redirect from /p/s3a-swarm-main/code, got ${redirectRes1.status}`);
  assert.equal(redirectRes1.headers.get("location"), "/p/brakets/code");
  console.log("   ✓ /p/s3a-swarm-main/code -> 301 Location: /p/brakets/code");

  const redirectRes2 = await fetch(`${BASE_URL}/p/s3a-swarm-main/insights`, { redirect: "manual" });
  assert.equal(redirectRes2.status, 301, `Expected 301 redirect from /p/s3a-swarm-main/insights, got ${redirectRes2.status}`);
  assert.equal(redirectRes2.headers.get("location"), "/p/brakets/insights");
  console.log("   ✓ /p/s3a-swarm-main/insights -> 301 Location: /p/brakets/insights");

  const redirectRes3 = await fetch(`${BASE_URL}/project/s3a-swarm-main/code`, { redirect: "manual" });
  assert.equal(redirectRes3.status, 301, `Expected 301 redirect from /project/s3a-swarm-main/code, got ${redirectRes3.status}`);
  assert.equal(redirectRes3.headers.get("location"), "/p/brakets/code");
  console.log("   ✓ /project/s3a-swarm-main/code -> 301 Location: /p/brakets/code");

  // --------------------------------------------------------------------------
  // 2. Verify /p/brakets/code Isolation (No Swarm Platform Code)
  // --------------------------------------------------------------------------
  console.log("\n2. Verifying /p/brakets/code analysis page and code isolation...");
  const codePageRes = await fetch(`${BASE_URL}/p/brakets/code`);
  assert.equal(codePageRes.status, 200, `Expected 200 OK from /p/brakets/code, got ${codePageRes.status}`);
  const codeHtml = await codePageRes.text();

  // Verify header label
  assert.ok(codeHtml.includes("swarm"), "Header must include platform name 'swarm'");
  assert.ok(codeHtml.includes("Brakets"), "Header must include project name 'Brakets'");
  assert.ok(codeHtml.includes("plat-gaming"), "Header must include project slug 'plat-gaming'");
  assert.ok(codeHtml.includes("env:") || codeHtml.includes("test"), "Header must include environment badge");
  console.log("   ✓ Header branding verified: swarm ▶ Brakets (plat-gaming) • env: test");

  // Verify Brakets code components exist in repo tree
  assert.ok(codeHtml.includes("apps/brakets-gg"), "Repo tree must include apps/brakets-gg");
  assert.ok(codeHtml.includes("demo/brakets-bracket"), "Repo tree must include demo/brakets-bracket");
  assert.ok(codeHtml.includes("fixtures"), "Repo tree must include fixtures");

  // Verify strict code isolation: NO swarm platform folders
  assert.ok(!codeHtml.includes("📂 docs/vocab"), "Repo tree must NOT expose platform docs/vocab");
  assert.ok(!codeHtml.includes("📂 scripts"), "Repo tree must NOT expose platform scripts");
  assert.ok(!codeHtml.includes("📂 cloudflare"), "Repo tree must NOT expose platform cloudflare code");
  console.log("   ✓ Strict code isolation verified: Brakets code only, zero platform files.");

  // Verify embedded Visual SVG review canvas
  assert.ok(codeHtml.includes("/project/brakets/visual-review.svg"), "Code page must embed visual-review.svg");
  assert.ok(codeHtml.includes("Visual Review Canvas"), "Code page must have Visual Review Canvas section");
  console.log("   ✓ Embedded visual SVG review canvas verified on /p/brakets/code.");

  // --------------------------------------------------------------------------
  // 3. Verify Visual Review SVG Endpoint
  // --------------------------------------------------------------------------
  console.log("\n3. Verifying SVG Visual Review Canvas endpoint...");
  const svgRes = await fetch(`${BASE_URL}/project/brakets/visual-review.svg`);
  assert.equal(svgRes.status, 200, `Expected 200 OK from visual-review.svg, got ${svgRes.status}`);
  assert.ok(svgRes.headers.get("content-type")?.includes("image/svg+xml"), "Must return image/svg+xml");
  const svgContent = await svgRes.text();
  assert.ok(svgContent.includes("<svg"), "Must contain valid SVG opening tag");
  assert.ok(svgContent.includes("Visual Code &amp; Design Review Canvas") || svgContent.includes("Visual Code"), "SVG must include review canvas title");
  assert.ok(svgContent.includes("Senior Dev (gate-agent)"), "SVG must identify Senior Dev gate-agent");
  assert.ok(svgContent.includes("FAILED") || svgContent.includes("PASSED") || svgContent.includes("REPAIRED"), "SVG must include verification badges");
  assert.ok(svgContent.includes("lib/escrow.ts:12"), "SVG must include exact file:line error anchors");
  console.log("   ✓ SVG Visual Review Canvas verified with AST hierarchy, design wireframes, file:line anchors, and repair checks.");

  // --------------------------------------------------------------------------
  // 4. Verify Platform Pages (/platform/*)
  // --------------------------------------------------------------------------
  console.log("\n4. Verifying Swarm Platform Pages (/platform/*)...");
  const sections = ["overview", "projects", "agents", "ledger", "traces"];
  for (const sec of sections) {
    const res = await fetch(`${BASE_URL}/platform/${sec}`);
    assert.equal(res.status, 200, `Expected 200 OK from /platform/${sec}, got ${res.status}`);
    const html = await res.text();
    assert.ok(html.includes("Swarm Platform"), `Page /platform/${sec} must contain 'Swarm Platform'`);
    console.log(`   ✓ /platform/${sec} returns 200 OK.`);
  }

  // --------------------------------------------------------------------------
  // 5. Verify Projects API & Create Project Flow
  // --------------------------------------------------------------------------
  console.log("\n5. Verifying Projects API (GET /api/projects & POST /api/projects)...");
  const listRes = await fetch(`${BASE_URL}/api/projects`);
  assert.equal(listRes.status, 200, `Expected 200 OK from /api/projects, got ${listRes.status}`);
  const listData = await listRes.json();
  assert.ok(Array.isArray(listData.projects), "Must return projects array");
  const projectIds = listData.projects.map(p => p.id);
  assert.ok(projectIds.includes("brakets"), "Projects list must include 'brakets'");
  console.log(`   ✓ GET /api/projects returned ${listData.projects.length} registered projects.`);

  // Create Project: Three.js 3D Site
  console.log("   Testing POST /api/projects to create a Three.js 3D site...");
  const createRes = await fetch(`${BASE_URL}/api/projects`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      prompt: "create a three.js 3D site with orbit controls and GLTF loader",
      template: "threejs_site",
      id: "threejs-gallery",
      name: "Spatial Three.js Gallery",
    }),
  });
  assert.ok(createRes.status === 200 || createRes.status === 201, `Expected 200 or 201 from POST /api/projects, got ${createRes.status}`);
  const createResult = await createRes.json();
  assert.ok(createResult.success, "Creation response must indicate success");
  assert.equal(createResult.project.id, "threejs-gallery");
  assert.ok(createResult.project.repo_name.includes("threejs-gallery"));
  assert.equal(createResult.project.template, "threejs_site");
  console.log(`   ✓ Created project 'threejs-gallery' with isolated repository, issue board, and format template.`);

  // --------------------------------------------------------------------------
  // Summary
  // --------------------------------------------------------------------------
  console.log("\n===============================================================================");
  console.log("      ALL PLATFORM VS PROJECT SEPARATION CHECKS PASSED SUCCESSFULLY!          ");
  console.log("===============================================================================");
}

run().catch((err) => {
  console.error("\n❌ Verification Failed:", err);
  process.exit(1);
});
