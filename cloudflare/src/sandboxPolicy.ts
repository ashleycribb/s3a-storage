// cloudflare/src/sandboxPolicy.ts
// Cost-control rules for Cloudflare Sandbox containers. Pure functions with no SDK import so
// they can be unit tested with `npm test`.
//
// Why this exists: a standard-1 container bills memory and disk the whole time it runs, whether
// or not it is doing work. A shared sandbox that every scheduler tick (every 5 minutes) touches
// never reaches its idle timeout, so it stays on. Rules:
//   1. Each agent task gets its own sandbox ID and the sandbox is destroyed when the task ends.
//   2. Every getSandbox() call sets a short sleepAfter (SANDBOX_SLEEP_AFTER).
//   3. Nothing starts a container unasked: diagnostics need ?wake=1 plus admin auth, the live
//      preview proxy only forwards to a container that is already running, and the automatic
//      rebuild after conflict resolution is off unless AUTO_PREVIEW_REBUILD=true.

/** Idle time before a sandbox sleeps. The SDK default is 10m; the old preview path used 15m. */
export const SANDBOX_SLEEP_AFTER = "2m";

function sanitize(part: string, max: number): string {
  return String(part || "x")
    .toLowerCase()
    .replace(/[^a-z0-9_-]/g, "-")
    .slice(0, max) || "x";
}

/** One sandbox per agent task run. Lowercase, DNS-safe, at most 63 characters. */
export function taskSandboxId(projectId: string, taskId: string, opId?: string): string {
  const parts = ["s3a-task", sanitize(projectId, 16), sanitize(taskId, 16)];
  if (opId) parts.push(sanitize(opId, 20));
  return parts.join("-").slice(0, 63);
}

/** The long-lived live preview sandbox for a project (explicitly started by a person). */
export function previewSandboxId(projectId: string): string {
  return `s3a-preview-${sanitize(projectId, 32)}-v6`;
}

/** A throwaway sandbox for /test-sandbox diagnostics, never shared with the preview. */
export function diagSandboxId(now: number = Date.now()): string {
  return `s3a-diag-${now.toString(36)}`;
}

/** Diagnostics start a container, so the caller must ask for it explicitly: ?wake=1. */
export function wakeRequested(url: URL): boolean {
  return url.searchParams.get("wake") === "1";
}

/** The automatic preview rebuild after conflict resolution starts a container; opt in only. */
export function autoPreviewRebuildEnabled(env: { AUTO_PREVIEW_REBUILD?: string }): boolean {
  return String(env.AUTO_PREVIEW_REBUILD || "").toLowerCase() === "true";
}

export interface DestroyableSandbox {
  destroy(): Promise<void>;
}

/**
 * Run `fn` with a per-task sandbox and always destroy it afterwards, even if `fn` throws.
 * `getSandbox` is injected so tests can use a fake.
 */
export async function withTaskSandbox<S extends DestroyableSandbox, T>(
  getSandbox: (ns: any, id: string, options?: any) => S,
  namespace: any,
  sandboxId: string,
  fn: (sandbox: S) => Promise<T>,
  options: Record<string, unknown> = {},
  onDestroyError: (err: unknown) => void = (err) => console.error("sandbox destroy failed:", err)
): Promise<T> {
  const sandbox = getSandbox(namespace, sandboxId, {
    normalizeId: true,
    sleepAfter: SANDBOX_SLEEP_AFTER,
    ...options,
  });
  try {
    return await fn(sandbox);
  } finally {
    try {
      await sandbox.destroy();
    } catch (err) {
      onDestroyError(err);
    }
  }
}
