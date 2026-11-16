// Acceptance test (written before the fix): the unauthenticated GET /control must expose only
// `paused` and `status` (security hardening, c2b3b68). eb6a9ad added paused_by / paused_reason /
// paused_at there, which reveals the supervisor's identity to anyone and invents values
// ("Supervisor (Ashley)", paused_at = now) when nothing was stored.
import { test } from "node:test";
import assert from "node:assert/strict";
import { publicControlView } from "../src/controlView.ts";

test("public control view has exactly paused and status", () => {
  const v = publicControlView({
    paused: true,
    paused_by: "Human Supervisor",
    paused_reason: "Manual pause via /control",
    paused_at: 1794790515729,
    scheduler_status: "active",
  }, Date.now());
  assert.deepEqual(Object.keys(v).sort(), ["paused", "status"]);
  assert.equal(v.paused, true);
  assert.equal(v.status, "active");
  assert.ok(!JSON.stringify(v).includes("Supervisor"), "no supervisor identity in the public view");
});

test("nothing stored: no invented identity or timestamp", () => {
  const v = publicControlView({ paused: false }, 1234);
  assert.deepEqual(Object.keys(v).sort(), ["paused", "status"]);
  assert.equal(v.paused, false);
  assert.equal(v.status, "idle");
});
