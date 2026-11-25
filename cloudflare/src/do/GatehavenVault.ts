// cloudflare/src/do/GatehavenVault.ts
// Gatehaven account vault: one Durable Object instance per player vault.
// Stores, per vaultId (isolated by DO instance):
//   - account: parent email / child name / age band / player code / token HASH
//     (the raw token is returned once at creation and never stored)
//   - state: the player's sealed cloud-save state, versioned, hash-chained
//   - ledger: append-only sealed event entries (quest/skill/dungeon/soulbind/level)
// The DO answers internal requests from src/routes/gatehaven.ts, which
// forwards the public /v1/gatehaven/* API to the per-vault instance.

import type { Env } from "../types";
import { canonicalJson, sha256Hex, GENESIS_HASH } from "../gatehaven/canonical.ts";

export interface GatehavenAccount {
  vaultId: string;
  parentEmail: string;
  childName: string;
  childAgeBand: string;
  playerCode: string;
  /** SHA-256 hex of the bearer token. The raw token is never stored. */
  tokenHash: string;
  createdAt: number;
}

export interface GatehavenSeal {
  version: number;
  prevHash: string;
  hash: string;
}

export interface GatehavenStateRecord {
  version: number;
  state: unknown;
  stateHash: string;
  seal: GatehavenSeal;
  updatedAt: number;
}

export interface GatehavenLedgerEntry {
  seq: number;
  type: string;
  data: unknown;
  at: number;
  prevHash: string;
  entryHash: string;
}

const AGE_BANDS = new Set(["under-8", "8-12", "13-17"]);

const EVENT_TYPES = new Set([
  "quest_completed",
  "skill_learned",
  "dungeon_cleared",
  "item_soulbound",
  "level_gained",
]);

// Uppercase consonants + digits, no ambiguous chars (0/O/1/I) and no vowels
// (so codes never spell words by accident).
const CODE_ALPHABET = "BCDFGHJKLMNPQRSTVWXZ23456789";

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

function jsonResponse(data: unknown, status = 200): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function errorResponse(error: string, status: number): Response {
  return jsonResponse({ error }, status);
}

function generatePlayerCode(): string {
  const bytes = new Uint8Array(6);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => CODE_ALPHABET[b % CODE_ALPHABET.length]).join("");
}

function generateToken(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export class GatehavenVault implements DurableObject {
  state: DurableObjectState;
  env: Env;

  constructor(state: DurableObjectState, env: Env) {
    this.state = state;
    this.env = env;
  }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const parts = url.pathname.split("/").filter(Boolean);
    // Internal paths: /vault/:vaultId/account | state | events | ledger
    if (parts[0] !== "vault" || parts.length !== 3) {
      return errorResponse("Not found", 404);
    }
    const vaultId = parts[1];
    const sub = parts[2];

    try {
      if (sub === "account" && request.method === "POST") {
        return await this.createAccount(request, vaultId);
      }
      if (sub === "state" && request.method === "GET") {
        return await this.getState(request, vaultId);
      }
      if (sub === "state" && request.method === "PUT") {
        return await this.putState(request, vaultId);
      }
      if (sub === "events" && request.method === "POST") {
        return await this.postEvent(request, vaultId);
      }
      if (sub === "ledger" && request.method === "GET") {
        return await this.getLedger(request, vaultId);
      }
      return errorResponse("Not found", 404);
    } catch (err: any) {
      return errorResponse(err?.message || String(err), 500);
    }
  }

  // --------------------------------------------------------------------------
  // Helpers
  // --------------------------------------------------------------------------
  private async readJsonBody(request: Request): Promise<{ ok: true; body: unknown } | { ok: false; response: Response }> {
    let body: unknown;
    try {
      body = await request.json();
    } catch {
      return { ok: false, response: errorResponse("Invalid JSON body", 400) };
    }
    return { ok: true, body };
  }

  private async loadAccount(vaultId: string): Promise<GatehavenAccount | null> {
    const account = await this.state.storage.get<GatehavenAccount>("account");
    if (!account || account.vaultId !== vaultId) return null;
    return account;
  }

  /** Returns the account on success, or the 404/401 Response to send. */
  private async authorize(
    request: Request,
    vaultId: string,
  ): Promise<{ ok: true; account: GatehavenAccount } | { ok: false; response: Response }> {
    const account = await this.loadAccount(vaultId);
    if (!account) {
      return { ok: false, response: errorResponse("Vault not found", 404) };
    }
    const auth = request.headers.get("Authorization") || "";
    const token = auth.startsWith("Bearer ") ? auth.slice(7).trim() : "";
    if (!token) {
      return { ok: false, response: errorResponse("Missing bearer token", 401) };
    }
    const tokenHash = await sha256Hex(token);
    if (tokenHash !== account.tokenHash) {
      return { ok: false, response: errorResponse("Invalid token", 401) };
    }
    return { ok: true, account };
  }

  // --------------------------------------------------------------------------
  // POST /vault/:vaultId/account — create the vault's account (once)
  // --------------------------------------------------------------------------
  private async createAccount(request: Request, vaultId: string): Promise<Response> {
    const parsed = await this.readJsonBody(request);
    if (!parsed.ok) return parsed.response;
    const body = parsed.body;
    if (!isPlainObject(body)) {
      return errorResponse("Body must be a JSON object", 400);
    }
    const { parentEmail, childName, childAgeBand } = body;
    if (typeof parentEmail !== "string" || !EMAIL_RE.test(parentEmail)) {
      return errorResponse("parentEmail must be a valid email address", 400);
    }
    if (typeof childName !== "string" || childName.trim().length === 0 || childName.trim().length > 40) {
      return errorResponse("childName must be non-empty and at most 40 characters", 400);
    }
    if (typeof childAgeBand !== "string" || !AGE_BANDS.has(childAgeBand)) {
      return errorResponse("childAgeBand must be one of under-8, 8-12, 13-17", 400);
    }
    const existing = await this.loadAccount(vaultId);
    if (existing) {
      return errorResponse("Vault already has an account", 409);
    }

    const token = generateToken();
    const account: GatehavenAccount = {
      vaultId,
      parentEmail,
      childName: childName.trim(),
      childAgeBand,
      playerCode: generatePlayerCode(),
      tokenHash: await sha256Hex(token),
      createdAt: Date.now(),
    };
    await this.state.storage.put("account", account);
    return jsonResponse(
      {
        vaultId,
        gateAddress: `gate://gatehaven/vault/${vaultId}`,
        playerCode: account.playerCode,
        token,
      },
      201,
    );
  }

  // --------------------------------------------------------------------------
  // GET /vault/:vaultId/state
  // --------------------------------------------------------------------------
  private async getState(request: Request, vaultId: string): Promise<Response> {
    const auth = await this.authorize(request, vaultId);
    if (!auth.ok) return auth.response;
    const record = await this.state.storage.get<GatehavenStateRecord>("state");
    if (!record) {
      return jsonResponse({
        vaultId,
        version: 0,
        state: null,
        stateHash: null,
        seal: null,
        updatedAt: null,
      });
    }
    return jsonResponse({
      vaultId,
      version: record.version,
      state: record.state,
      stateHash: record.stateHash,
      seal: record.seal,
      updatedAt: record.updatedAt,
    });
  }

  // --------------------------------------------------------------------------
  // PUT /vault/:vaultId/state — verify-before-save + version guard
  // --------------------------------------------------------------------------
  private async putState(request: Request, vaultId: string): Promise<Response> {
    const auth = await this.authorize(request, vaultId);
    if (!auth.ok) return auth.response;
    const parsed = await this.readJsonBody(request);
    if (!parsed.ok) return parsed.response;
    const body = parsed.body;
    if (!isPlainObject(body)) {
      return errorResponse("Body must be a JSON object", 400);
    }
    const { state, stateHash, baseVersion } = body;
    if (!isPlainObject(state)) {
      return errorResponse("state must be a JSON object", 400);
    }
    if (typeof stateHash !== "string" || !/^[0-9a-f]{64}$/.test(stateHash)) {
      return errorResponse("stateHash must be a SHA-256 hex string", 400);
    }
    if (typeof baseVersion !== "number" || !Number.isInteger(baseVersion) || baseVersion < 0) {
      return errorResponse("baseVersion must be a non-negative integer", 400);
    }

    // Verify-before-save: recompute the hash server-side; never store
    // unverified state.
    const computed = await sha256Hex(canonicalJson(state));
    if (computed !== stateHash) {
      return errorResponse("stateHash does not match state", 400);
    }

    const current = await this.state.storage.get<GatehavenStateRecord>("state");
    const currentVersion = current?.version ?? 0;
    if (baseVersion < currentVersion) {
      // Never overwrite a newer sealed version with an older one.
      return jsonResponse(
        {
          error: "Version conflict: baseVersion is stale",
          version: currentVersion,
          stateHash: current!.stateHash,
          seal: current!.seal,
        },
        409,
      );
    }
    if (baseVersion > currentVersion) {
      return errorResponse("baseVersion is ahead of the stored version", 400);
    }

    const prevHash = current?.seal.hash ?? GENESIS_HASH;
    const version = currentVersion + 1;
    const seal: GatehavenSeal = {
      version,
      prevHash,
      hash: await sha256Hex(canonicalJson({ version, prevHash, stateHash })),
    };
    const record: GatehavenStateRecord = {
      version,
      state,
      stateHash,
      seal,
      updatedAt: Date.now(),
    };
    await this.state.storage.put("state", record);
    return jsonResponse({
      version,
      stateHash,
      seal,
      updatedAt: record.updatedAt,
    });
  }

  // --------------------------------------------------------------------------
  // POST /vault/:vaultId/events — append one sealed ledger entry
  // --------------------------------------------------------------------------
  private async postEvent(request: Request, vaultId: string): Promise<Response> {
    const auth = await this.authorize(request, vaultId);
    if (!auth.ok) return auth.response;
    const parsed = await this.readJsonBody(request);
    if (!parsed.ok) return parsed.response;
    const body = parsed.body;
    if (!isPlainObject(body)) {
      return errorResponse("Body must be a JSON object", 400);
    }
    const { type, data } = body;
    if (typeof type !== "string" || !EVENT_TYPES.has(type)) {
      return errorResponse(
        "type must be one of quest_completed, skill_learned, dungeon_cleared, item_soulbound, level_gained",
        400,
      );
    }
    if (!("data" in body)) {
      return errorResponse("data is required", 400);
    }

    const ledger = (await this.state.storage.get<GatehavenLedgerEntry[]>("ledger")) ?? [];
    const prevHash = ledger.length > 0 ? ledger[ledger.length - 1].entryHash : GENESIS_HASH;
    const seq = ledger.length + 1;
    const at = Date.now();
    const entry: GatehavenLedgerEntry = {
      seq,
      type,
      data,
      at,
      prevHash,
      entryHash: await sha256Hex(canonicalJson({ seq, type, data, at, prevHash })),
    };
    ledger.push(entry);
    await this.state.storage.put("ledger", ledger);
    return jsonResponse({ seq, entryHash: entry.entryHash, at }, 201);
  }

  // --------------------------------------------------------------------------
  // GET /vault/:vaultId/ledger — entries in seq order
  // --------------------------------------------------------------------------
  private async getLedger(request: Request, vaultId: string): Promise<Response> {
    const auth = await this.authorize(request, vaultId);
    if (!auth.ok) return auth.response;
    const ledger = (await this.state.storage.get<GatehavenLedgerEntry[]>("ledger")) ?? [];
    return jsonResponse({
      vaultId,
      entries: ledger.map((e) => ({
        seq: e.seq,
        type: e.type,
        data: e.data,
        at: e.at,
        entryHash: e.entryHash,
      })),
    });
  }
}
