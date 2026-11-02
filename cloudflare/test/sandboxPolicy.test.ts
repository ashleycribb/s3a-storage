// Run with: npm test (node --test). Pure policy tests, no Cloudflare runtime needed.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  SANDBOX_SLEEP_AFTER,
  taskSandboxId,
  previewSandboxId,
  diagSandboxId,
  wakeRequested,
  autoPreviewRebuildEnabled,
  withTaskSandbox,
} from "../src/sandboxPolicy.ts";

test("sleepAfter is 1-2 minutes", () => {
  assert.match(SANDBOX_SLEEP_AFTER, /^[12]m$/);
});

test("each task and run gets its own DNS-safe sandbox id", () => {
  const a = taskSandboxId("s3a-swarm-main", "TASK-12", "op_sched_task-12_1");
  const b = taskSandboxId("s3a-swarm-main", "TASK-13", "op_sched_task-13_1");
  const c = taskSandboxId("s3a-swarm-main", "TASK-12", "op_sched_task-12_2");
  assert.notEqual(a, b);
  assert.notEqual(a, c);
  for (const id of [a, b, c]) {
    assert.match(id, /^[a-z0-9-_]+$/);
    assert.ok(id.length <= 63);
    assert.ok(id.startsWith("s3a-task-"));
  }
  assert.notEqual(a, previewSandboxId("s3a-swarm-main"));
});

test("hostile ids are sanitised", () => {
  const id = taskSandboxId("../../etc", "TASK 1; rm -rf /", "op$$");
  assert.match(id, /^[a-z0-9-_]+$/);
});

test("diagnostics sandboxes are never the preview sandbox", () => {
  assert.notEqual(diagSandboxId(1), previewSandboxId("s3a-swarm-main"));
  assert.notEqual(diagSandboxId(1), diagSandboxId(2));
});

test("wake needs an explicit ?wake=1", () => {
  assert.equal(wakeRequested(new URL("https://x/test-sandbox")), false);
  assert.equal(wakeRequested(new URL("https://x/test-sandbox?wake=0")), false);
  assert.equal(wakeRequested(new URL("https://x/test-sandbox?wake=1")), true);
});

test("auto preview rebuild is off unless AUTO_PREVIEW_REBUILD=true", () => {
  assert.equal(autoPreviewRebuildEnabled({}), false);
  assert.equal(autoPreviewRebuildEnabled({ AUTO_PREVIEW_REBUILD: "false" }), false);
  assert.equal(autoPreviewRebuildEnabled({ AUTO_PREVIEW_REBUILD: "true" }), true);
});

function fakeGetSandbox(log: any[], destroyThrows = false) {
  return (ns: any, id: string, options?: any) => {
    log.push({ op: "get", id, options });
    return {
      async destroy() {
        log.push({ op: "destroy", id });
        if (destroyThrows) throw new Error("boom");
      },
    };
  };
}

test("withTaskSandbox sets sleepAfter and destroys after success", async () => {
  const log: any[] = [];
  const out = await withTaskSandbox(fakeGetSandbox(log), {}, "s3a-task-x", async () => "ok");
  assert.equal(out, "ok");
  assert.equal(log[0].options.sleepAfter, SANDBOX_SLEEP_AFTER);
  assert.equal(log[0].options.normalizeId, true);
  assert.deepEqual(log.at(-1), { op: "destroy", id: "s3a-task-x" });
});

test("withTaskSandbox destroys even when the task throws", async () => {
  const log: any[] = [];
  await assert.rejects(
    withTaskSandbox(fakeGetSandbox(log), {}, "s3a-task-y", async () => {
      throw new Error("build blew up");
    }),
    /build blew up/
  );
  assert.equal(log.filter((l) => l.op === "destroy").length, 1);
});

test("a failing destroy is reported but does not hide the result", async () => {
  const log: any[] = [];
  const errors: unknown[] = [];
  const out = await withTaskSandbox(fakeGetSandbox(log, true), {}, "s3a-task-z", async () => 42, {}, (e) => errors.push(e));
  assert.equal(out, 42);
  assert.equal(errors.length, 1);
});

// Source guards: stop the regressions that caused the always-on cost.
const src = (p: string) => readFileSync(new URL(`../src/${p}`, import.meta.url), "utf8");

test("every direct getSandbox() call sets sleepAfter; the rest go through withTaskSandbox", () => {
  for (const file of ["do/RepoCoordinator.ts", "workflow/AgentTask.ts", "routes/api.ts"]) {
    const text = src(file);
    for (const m of text.matchAll(/=\s*getSandbox\(/g)) {
      const window = text.slice(m.index!, m.index! + 400);
      assert.match(window, /sleepAfter/, `${file}: getSandbox() without sleepAfter`);
    }
  }
});

test("sandbox_build callers use a per-task sandbox, not the shared preview id", () => {
  for (const file of ["do/RepoCoordinator.ts", "workflow/AgentTask.ts"]) {
    const text = src(file);
    assert.match(text, /withTaskSandbox\(getSandbox, this\.env\.Sandbox, taskSandboxId\(/);
    assert.doesNotMatch(text, /getSandbox\(this\.env\.Sandbox, `s3a-preview-/);
  }
});

test("forwardToPort never starts a container", () => {
  const text = src("sandbox.ts");
  const fn = text.slice(text.indexOf("async forwardToPort"), text.indexOf("(S3ASandbox as any).outbound"));
  assert.doesNotMatch(fn, /container\.start\(/);
  assert.doesNotMatch(fn, /containerFetch/);
  assert.match(fn, /container\.running/);
});

test("package pins @cloudflare/sandbox to the image version and wrangler caps instances", () => {
  const pkg = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8"));
  assert.equal(pkg.dependencies["@cloudflare/sandbox"], "0.12.10");
  const wr = readFileSync(new URL("../wrangler.toml", import.meta.url), "utf8");
  assert.match(wr, /^max_instances = 2$/m);
  assert.match(wr, /sandbox:0\.12\.10/);
});
