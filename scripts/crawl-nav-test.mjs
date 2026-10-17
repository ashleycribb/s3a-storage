// scripts/crawl-nav-test.mjs
// Crawls every navigation link, tab, breadcrumb, and deep route from every page.
// Verifies HTTP 200 (no 404s), 301 redirects for retired paths, shared header/nav, and zero iframes.

import assert from "node:assert";

const BASE_URL = process.env.BASE_URL || "https://s3a-gate.ashley-e-cribb.workers.dev";
const PROJECT_ID = "s3a-swarm-main";

console.log(`\n======================================================`);
console.log(`🕷️ S3A Gate UI Navigation Crawler & Route Verification`);
console.log(`Target: ${BASE_URL}`);
console.log(`Project: ${PROJECT_ID}`);
console.log(`======================================================\n`);

let passedChecks = 0;
let totalChecks = 0;

function check(name, condition, msg) {
  totalChecks++;
  if (condition) {
    passedChecks++;
    console.log(`  ✅ ${name}`);
  } else {
    console.error(`  ❌ ${name}: ${msg || "Assertion failed"}`);
    throw new Error(`${name}: ${msg}`);
  }
}

// --------------------------------------------------------------------------
// 1. Verify 301 Redirects for Retired Routes and ?tab= Query Params
// --------------------------------------------------------------------------
console.log(`[SECTION 1] Testing 301 Redirects for Retired Routes & ?tab= Parameters...`);

const redirectTests = [
  { url: `${BASE_URL}/`, expectedLocation: `/p/${PROJECT_ID}/code` },
  { url: `${BASE_URL}/?tab=code`, expectedLocation: `/p/${PROJECT_ID}/code` },
  { url: `${BASE_URL}/?tab=issues`, expectedLocation: `/p/${PROJECT_ID}/issues` },
  { url: `${BASE_URL}/?tab=pulls`, expectedLocation: `/p/${PROJECT_ID}/pulls` },
  { url: `${BASE_URL}/?tab=agents`, expectedLocation: `/p/${PROJECT_ID}/agents` },
  { url: `${BASE_URL}/?tab=actions`, expectedLocation: `/p/${PROJECT_ID}/actions` },
  { url: `${BASE_URL}/?tab=insights`, expectedLocation: `/p/${PROJECT_ID}/insights` },
  { url: `${BASE_URL}/preview/cr/CR-101`, expectedLocation: `/p/${PROJECT_ID}/pulls/CR-101` },
  { url: `${BASE_URL}/code`, expectedLocation: `/p/${PROJECT_ID}/code` },
  { url: `${BASE_URL}/issues`, expectedLocation: `/p/${PROJECT_ID}/issues` },
  { url: `${BASE_URL}/pulls`, expectedLocation: `/p/${PROJECT_ID}/pulls` },
  { url: `${BASE_URL}/agents`, expectedLocation: `/p/${PROJECT_ID}/agents` },
  { url: `${BASE_URL}/actions`, expectedLocation: `/p/${PROJECT_ID}/actions` },
  { url: `${BASE_URL}/insights`, expectedLocation: `/p/${PROJECT_ID}/insights` },
  { url: `${BASE_URL}/p/${PROJECT_ID}`, expectedLocation: `/p/${PROJECT_ID}/code` },
];

for (const t of redirectTests) {
  const res = await fetch(t.url, { redirect: "manual" });
  const location = res.headers.get("location") || "";
  check(
    `Redirect ${t.url} -> 301 to ${t.expectedLocation}`,
    res.status === 301 && location.endsWith(t.expectedLocation),
    `Got status ${res.status}, location: ${location}`
  );
}

// --------------------------------------------------------------------------
// 2. Comprehensive Navigation & Page Crawl
// --------------------------------------------------------------------------
console.log(`\n[SECTION 2] Crawling Every Nav Link & Route from Every Page...`);

const visitedUrls = new Set();
const urlsToVisit = [
  `/p/${PROJECT_ID}/code`,
  `/p/${PROJECT_ID}/issues`,
  `/p/${PROJECT_ID}/pulls`,
  `/p/${PROJECT_ID}/agents`,
  `/p/${PROJECT_ID}/actions`,
  `/p/${PROJECT_ID}/insights`,
  `/p/${PROJECT_ID}/issues/GH-3`,
  `/p/${PROJECT_ID}/issues/TASK-NONEXISTENT`,
  `/p/${PROJECT_ID}/pulls/CR-NONEXISTENT`,
  `/p/${PROJECT_ID}/actions/RUN-NONEXISTENT`,
];

const crawlResults = [];

while (urlsToVisit.length > 0) {
  const relativePath = urlsToVisit.shift();
  if (visitedUrls.has(relativePath)) continue;
  visitedUrls.add(relativePath);

  const fullUrl = `${BASE_URL}${relativePath}`;
  const res = await fetch(fullUrl, { redirect: "manual" });
  const html = await res.text();

  crawlResults.push({
    path: relativePath,
    status: res.status,
    contentType: res.headers.get("content-type") || "",
  });

  // Verify status is 200 (NO 404)
  check(
    `GET ${relativePath} returns 200 (not 404)`,
    res.status === 200,
    `Got status ${res.status}`
  );

  // Verify Shared Shell components
  check(
    `${relativePath}: Contains shared header & logo`,
    html.includes("S3A Gate") && html.includes('class="gh-header"'),
    `Missing shared header`
  );

  check(
    `${relativePath}: Contains all 6 nav tabs`,
    html.includes(`/p/${PROJECT_ID}/code`) &&
    html.includes(`/p/${PROJECT_ID}/issues`) &&
    html.includes(`/p/${PROJECT_ID}/pulls`) &&
    html.includes(`/p/${PROJECT_ID}/agents`) &&
    html.includes(`/p/${PROJECT_ID}/actions`) &&
    html.includes(`/p/${PROJECT_ID}/insights`),
    `Missing one or more navigation tabs`
  );

  check(
    `${relativePath}: Contains breadcrumb trail`,
    html.includes(PROJECT_ID),
    `Missing breadcrumb for ${PROJECT_ID}`
  );

  // Verify requirement: Preview is a link, NOT an iframe
  check(
    `${relativePath}: Zero iframe elements (preview is link, not iframe)`,
    !html.toLowerCase().includes("<iframe"),
    `Found prohibited iframe in response!`
  );

  // Extract all <a href="..."> links from the page
  const linkMatches = html.matchAll(/href=["'](\/p\/[^"']+)["']/g);
  for (const match of linkMatches) {
    const rawLink = match[1];
    // Strip hash or query params
    const cleanLink = rawLink.split("?")[0].split("#")[0];
    if (cleanLink.startsWith(`/p/${PROJECT_ID}`) && !visitedUrls.has(cleanLink) && !urlsToVisit.includes(cleanLink)) {
      urlsToVisit.push(cleanLink);
    }
  }
}

// --------------------------------------------------------------------------
// 3. Crawl Summary Report
// --------------------------------------------------------------------------
console.log(`\n======================================================`);
console.log(`📊 Crawl Execution Summary`);
console.log(`======================================================`);
console.table(crawlResults);

console.log(`\nTotal Checked URLs: ${crawlResults.length}`);
console.log(`HTTP 200 OK: ${crawlResults.filter(r => r.status === 200).length}`);
console.log(`HTTP 404 Not Found: ${crawlResults.filter(r => r.status === 404).length}`);
console.log(`Total Assertions Passed: ${passedChecks}/${totalChecks}`);
console.log(`======================================================\n`);

if (crawlResults.some(r => r.status === 404)) {
  console.error("❌ Crawler detected 404 Not Found on one or more navigation routes!");
  process.exit(1);
}

if (passedChecks !== totalChecks) {
  console.error("❌ Some verification checks failed!");
  process.exit(1);
}

console.log(`🎉 All navigation links and routes passed with 200 OK!`);
