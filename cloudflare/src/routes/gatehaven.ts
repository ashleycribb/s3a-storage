// cloudflare/src/routes/gatehaven.ts
// Gatehaven account API v1: accounts, sealed cloud-save state, sealed ledger.
// Public routes (forwarded to the per-vault GatehavenVault Durable Object):
//   POST /v1/gatehaven/accounts
//   GET  /v1/gatehaven/vaults/:vaultId/state
//   PUT  /v1/gatehaven/vaults/:vaultId/state
//   POST /v1/gatehaven/vaults/:vaultId/events
//   GET  /v1/gatehaven/vaults/:vaultId/ledger
// Returns null for anything else so the worker falls through to the next
// router (existing routes are untouched).

import type { Env } from "../types";

export async function handleGatehavenRoutes(request: Request, env: Env): Promise<Response | null> {
  const url = new URL(request.url);
  const parts = url.pathname.split("/").filter(Boolean);
  if (parts[0] !== "v1" || parts[1] !== "gatehaven") {
    return null;
  }

  // POST /v1/gatehaven/accounts — the vaultId is minted here so the client
  // learns it from the response; the DO instance is keyed off it.
  if (parts.length === 3 && parts[2] === "accounts" && request.method === "POST") {
    const vaultId = crypto.randomUUID();
    return forwardToVault(request, env, vaultId, "account");
  }

  // /v1/gatehaven/vaults/:vaultId/{state,events,ledger}
  if (parts.length === 5 && parts[2] === "vaults") {
    const vaultId = parts[3];
    const sub = parts[4];
    if (sub === "state" && (request.method === "GET" || request.method === "PUT")) {
      return forwardToVault(request, env, vaultId, "state");
    }
    if (sub === "events" && request.method === "POST") {
      return forwardToVault(request, env, vaultId, "events");
    }
    if (sub === "ledger" && request.method === "GET") {
      return forwardToVault(request, env, vaultId, "ledger");
    }
  }

  return null;
}

async function forwardToVault(
  request: Request,
  env: Env,
  vaultId: string,
  sub: string,
): Promise<Response> {
  const stub = env.GATEHAVEN_VAULT.get(env.GATEHAVEN_VAULT.idFromName(`gatehaven-vault:${vaultId}`));
  const internalUrl = new URL(request.url);
  internalUrl.pathname = `/vault/${vaultId}/${sub}`;
  return stub.fetch(new Request(internalUrl.toString(), request));
}
