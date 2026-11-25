// Gatehaven account API v1 — test-first.
// Covers: account creation + validation, player-code format, token auth,
// state save/load with hash verification, version conflicts, ledger
// append order + hash chain + append-only, empty-state behaviour.
import { test } from "node:test";
import assert from "node:assert/strict";
import { handleGatehavenRoutes } from "../src/routes/gatehaven.ts";
import { GatehavenVault } from "../src/do/GatehavenVault.ts";
import { canonicalJson, sha256Hex } from "../src/gatehaven/canonical.ts";
import type { Env } from "../src/types.ts";

const GENESIS = "0".repeat(64);
const CODE_ALPHABET = "BCDFGHJKLMNPQRSTVWXZ23456789";

// ---------------------------------------------------------------------------
// Fakes: in-memory DurableObjectState storage + a fake GATEHAVEN_VAULT
// namespace that routes each vaultId to its own GatehavenVault instance.
// ---------------------------------------------------------------------------
function makeFakeStorage() {
  const map = new Map<string, unknown>();
  return {
    map,
    get: async (key: string) => map.get(key),
    put: async (key: string, value: unknown) => {
      map.set(key, value);
    },
    delete: async (key: string) => {
      map.delete(key);
    },
    list: async () => map,
  };
}

type FakeStorage = ReturnType<typeof makeFakeStorage>;

function makeVault(storage?: FakeStorage) {
  const s = storage ?? makeFakeStorage();
  const vault = new GatehavenVault(
    { storage: s } as unknown as DurableObjectState,
    {} as Env,
  );
  return { vault, storage: s };
}

function makeFakeEnv() {
  const vaults = new Map<string, { vault: GatehavenVault; storage: FakeStorage }>();
  const env = {
    GATEHAVEN_VAULT: {
      idFromName: (name: string) => name,
      get: (id: string) => {
        let entry = vaults.get(id);
        if (!entry) {
          entry = makeVault();
          vaults.set(id, entry);
        }
        return { fetch: (req: Request) => entry!.vault.fetch(req) };
      },
    },
    // Test-only handle so tests can inspect a vault's stored records.
    __vaults: vaults,
  };
  return env as unknown as Env;
}

async function createAccount(
  env: Env,
  body: Record<string, unknown> = {
    parentEmail: "parent@example.com",
    childName: "Ada",
    childAgeBand: "8-12",
  },
) {
  const res = await handleGatehavenRoutes(
    new Request("https://gate/v1/gatehaven/accounts", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
    env,
  );
  assert.ok(res, "handler must handle POST /v1/gatehaven/accounts");
  const data = (await res!.json()) as any;
  return { res: res!, data };
}

function authHeaders(token: string) {
  return { Authorization: `Bearer ${token}` };
}

// ---------------------------------------------------------------------------
// Account creation
// ---------------------------------------------------------------------------
test("account creation: good input returns vault, gate address, player code, token", async () => {
  const env = makeFakeEnv();
  const { res, data } = await createAccount(env);
  assert.equal(res.status, 201);
  assert.match(data.vaultId, /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/);
  assert.equal(data.gateAddress, `gate://gatehaven/vault/${data.vaultId}`);
  assert.equal(typeof data.playerCode, "string");
  assert.equal(data.playerCode.length, 6);
  for (const ch of data.playerCode) {
    assert.ok(CODE_ALPHABET.includes(ch), `player code char ${ch} must be unambiguous`);
  }
  assert.match(data.token, /^[0-9a-f]{64}$/);
});

test("account creation: token is stored only as a SHA-256 hash, never raw", async () => {
  const env = makeFakeEnv();
  const { data } = await createAccount(env);
  // Direct storage check: the stored account record holds the token hash,
  // and the raw token appears nowhere in it.
  const vaults = (env as any).__vaults as Map<
    string,
    { vault: GatehavenVault; storage: FakeStorage }
  >;
  const entry = vaults.get(`gatehaven-vault:${data.vaultId}`);
  assert.ok(entry, "vault entry exists");
  const account = entry!.storage.map.get("account") as any;
  assert.ok(account, "account record is stored");
  assert.equal(account.tokenHash, await sha256Hex(data.token));
  assert.ok(!JSON.stringify(account).includes(data.token), "raw token must not be stored");
  // Behaviour check: the raw token must never appear in responses, and a
  // wrong token must be rejected.
  const stateRes = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/state`, {
      headers: authHeaders(data.token),
    }),
    env,
  );
  assert.equal(stateRes!.status, 200);
  const body = await stateRes!.text();
  assert.ok(!body.includes(data.token), "raw token must never appear in responses");
  const wrong = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/state`, {
      headers: authHeaders("f".repeat(64)),
    }),
    env,
  );
  assert.equal(wrong!.status, 401);
});

test("account creation: bad email is rejected with 400", async () => {
  const env = makeFakeEnv();
  for (const bad of ["not-an-email", "a@b", "a b@c.com", "", 123]) {
    const { res } = await createAccount(env, {
      parentEmail: bad,
      childName: "Ada",
      childAgeBand: "8-12",
    });
    assert.equal(res.status, 400, `parentEmail ${JSON.stringify(bad)} must be 400`);
  }
});

test("account creation: childName must be non-empty and at most 40 chars", async () => {
  const env = makeFakeEnv();
  for (const bad of ["", "   ", "x".repeat(41)]) {
    const { res } = await createAccount(env, {
      parentEmail: "parent@example.com",
      childName: bad,
      childAgeBand: "8-12",
    });
    assert.equal(res.status, 400, `childName ${JSON.stringify(bad)} must be 400`);
  }
  const { res } = await createAccount(env, {
    parentEmail: "parent@example.com",
    childName: "x".repeat(40),
    childAgeBand: "8-12",
  });
  assert.equal(res.status, 201, "40-char childName is allowed");
});

test("account creation: childAgeBand must be one of the three bands", async () => {
  const env = makeFakeEnv();
  for (const bad of ["under-7", "18+", "adult", "", null]) {
    const { res } = await createAccount(env, {
      parentEmail: "parent@example.com",
      childName: "Ada",
      childAgeBand: bad,
    });
    assert.equal(res.status, 400, `childAgeBand ${JSON.stringify(bad)} must be 400`);
  }
  for (const good of ["under-8", "8-12", "13-17"]) {
    const { res } = await createAccount(env, {
      parentEmail: "parent@example.com",
      childName: "Ada",
      childAgeBand: good,
    });
    assert.equal(res.status, 201, `childAgeBand ${good} is allowed`);
  }
});

test("account creation: missing fields and invalid JSON are 400", async () => {
  const env = makeFakeEnv();
  const missing = await handleGatehavenRoutes(
    new Request("https://gate/v1/gatehaven/accounts", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ parentEmail: "parent@example.com" }),
    }),
    env,
  );
  assert.equal(missing!.status, 400);
  const badJson = await handleGatehavenRoutes(
    new Request("https://gate/v1/gatehaven/accounts", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: "{not json",
    }),
    env,
  );
  assert.equal(badJson!.status, 400);
});

test("account creation: two accounts get distinct vaults, codes, and tokens", async () => {
  const env = makeFakeEnv();
  const a = await createAccount(env);
  const b = await createAccount(env);
  assert.notEqual(a.data.vaultId, b.data.vaultId);
  assert.notEqual(a.data.token, b.data.token);
  assert.notEqual(a.data.playerCode, b.data.playerCode);
});

// ---------------------------------------------------------------------------
// Auth: 401 / 404
// ---------------------------------------------------------------------------
test("auth: missing or wrong token is 401; unknown vault is 404", async () => {
  const env = makeFakeEnv();
  const { data } = await createAccount(env);

  const noAuth = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/state`),
    env,
  );
  assert.equal(noAuth!.status, 401);

  const wrong = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/state`, {
      headers: authHeaders("a".repeat(64)),
    }),
    env,
  );
  assert.equal(wrong!.status, 401);

  const unknown = await handleGatehavenRoutes(
    new Request(
      `https://gate/v1/gatehaven/vaults/00000000-0000-4000-8000-000000000000/state`,
      { headers: authHeaders(data.token) },
    ),
    env,
  );
  assert.equal(unknown!.status, 404);

  const unknownLedger = await handleGatehavenRoutes(
    new Request(
      `https://gate/v1/gatehaven/vaults/00000000-0000-4000-8000-000000000000/ledger`,
      { headers: authHeaders(data.token) },
    ),
    env,
  );
  assert.equal(unknownLedger!.status, 404);
});

test("auth: a token from one vault does not work on another vault", async () => {
  const env = makeFakeEnv();
  const a = await createAccount(env);
  const b = await createAccount(env);
  const res = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${b.data.vaultId}/state`, {
      headers: authHeaders(a.data.token),
    }),
    env,
  );
  assert.equal(res!.status, 401);
});

// ---------------------------------------------------------------------------
// State: empty, round-trip, hash verification, version conflicts
// ---------------------------------------------------------------------------
test("state: fresh vault returns version 0 and null state with 200", async () => {
  const env = makeFakeEnv();
  const { data } = await createAccount(env);
  const res = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/state`, {
      headers: authHeaders(data.token),
    }),
    env,
  );
  assert.equal(res!.status, 200);
  const body = (await res!.json()) as any;
  assert.equal(body.vaultId, data.vaultId);
  assert.equal(body.version, 0);
  assert.equal(body.state, null);
});

test("state: save/load round-trip with hash verification and chained seal", async () => {
  const env = makeFakeEnv();
  const { data } = await createAccount(env);
  const state = { level: 3, xp: 120, gold: 45, gear: [{ id: "sword-1", soulbound: true }] };
  const stateHash = await sha256Hex(canonicalJson(state));

  const put = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/state`, {
      method: "PUT",
      headers: { ...authHeaders(data.token), "Content-Type": "application/json" },
      body: JSON.stringify({ state, stateHash, baseVersion: 0 }),
    }),
    env,
  );
  assert.equal(put!.status, 200);
  const saved = (await put!.json()) as any;
  assert.equal(saved.version, 1);
  assert.equal(saved.stateHash, stateHash);
  assert.equal(saved.seal.version, 1);
  assert.equal(saved.seal.prevHash, GENESIS);
  assert.equal(
    saved.seal.hash,
    await sha256Hex(canonicalJson({ version: 1, prevHash: GENESIS, stateHash })),
  );

  const get = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/state`, {
      headers: authHeaders(data.token),
    }),
    env,
  );
  assert.equal(get!.status, 200);
  const loaded = (await get!.json()) as any;
  assert.deepEqual(loaded.state, state);
  assert.equal(loaded.version, 1);
  assert.equal(loaded.stateHash, stateHash);
  assert.deepEqual(loaded.seal, saved.seal);
});

test("state: second save chains the seal to the previous hash", async () => {
  const env = makeFakeEnv();
  const { data } = await createAccount(env);
  const s1 = { level: 1 };
  const h1 = await sha256Hex(canonicalJson(s1));
  const put1 = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/state`, {
      method: "PUT",
      headers: { ...authHeaders(data.token), "Content-Type": "application/json" },
      body: JSON.stringify({ state: s1, stateHash: h1, baseVersion: 0 }),
    }),
    env,
  );
  const saved1 = (await put1!.json()) as any;

  const s2 = { level: 2 };
  const h2 = await sha256Hex(canonicalJson(s2));
  const put2 = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/state`, {
      method: "PUT",
      headers: { ...authHeaders(data.token), "Content-Type": "application/json" },
      body: JSON.stringify({ state: s2, stateHash: h2, baseVersion: 1 }),
    }),
    env,
  );
  assert.equal(put2!.status, 200);
  const saved2 = (await put2!.json()) as any;
  assert.equal(saved2.version, 2);
  assert.equal(saved2.seal.prevHash, saved1.seal.hash);
  assert.equal(
    saved2.seal.hash,
    await sha256Hex(canonicalJson({ version: 2, prevHash: saved1.seal.hash, stateHash: h2 })),
  );
});

test("state: hash mismatch is 400 and nothing is stored", async () => {
  const env = makeFakeEnv();
  const { data } = await createAccount(env);
  const state = { level: 1 };
  const wrongHash = "b".repeat(64);
  const put = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/state`, {
      method: "PUT",
      headers: { ...authHeaders(data.token), "Content-Type": "application/json" },
      body: JSON.stringify({ state, stateHash: wrongHash, baseVersion: 0 }),
    }),
    env,
  );
  assert.equal(put!.status, 400);
  const get = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/state`, {
      headers: authHeaders(data.token),
    }),
    env,
  );
  const loaded = (await get!.json()) as any;
  assert.equal(loaded.version, 0);
  assert.equal(loaded.state, null);
});

test("state: stale baseVersion is 409 and the stored state is unchanged", async () => {
  const env = makeFakeEnv();
  const { data } = await createAccount(env);
  const s1 = { level: 1 };
  const h1 = await sha256Hex(canonicalJson(s1));
  await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/state`, {
      method: "PUT",
      headers: { ...authHeaders(data.token), "Content-Type": "application/json" },
      body: JSON.stringify({ state: s1, stateHash: h1, baseVersion: 0 }),
    }),
    env,
  );

  const s2 = { level: 99 };
  const h2 = await sha256Hex(canonicalJson(s2));
  const stale = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/state`, {
      method: "PUT",
      headers: { ...authHeaders(data.token), "Content-Type": "application/json" },
      body: JSON.stringify({ state: s2, stateHash: h2, baseVersion: 0 }),
    }),
    env,
  );
  assert.equal(stale!.status, 409);
  const conflict = (await stale!.json()) as any;
  assert.equal(conflict.version, 1);
  assert.equal(conflict.stateHash, h1);

  const get = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/state`, {
      headers: authHeaders(data.token),
    }),
    env,
  );
  const loaded = (await get!.json()) as any;
  assert.equal(loaded.version, 1);
  assert.deepEqual(loaded.state, s1);
});

test("state: baseVersion ahead of the stored version is 400", async () => {
  const env = makeFakeEnv();
  const { data } = await createAccount(env);
  const state = { level: 1 };
  const stateHash = await sha256Hex(canonicalJson(state));
  const put = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/state`, {
      method: "PUT",
      headers: { ...authHeaders(data.token), "Content-Type": "application/json" },
      body: JSON.stringify({ state, stateHash, baseVersion: 5 }),
    }),
    env,
  );
  assert.equal(put!.status, 400);
});

// ---------------------------------------------------------------------------
// Ledger: append order, hash chain, append-only, validation
// ---------------------------------------------------------------------------
test("ledger: events append in order with a verifiable hash chain", async () => {
  const env = makeFakeEnv();
  const { data } = await createAccount(env);
  const types = ["quest_completed", "skill_learned", "dungeon_cleared"] as const;
  const hashes: string[] = [];
  let prev = GENESIS;
  for (let i = 0; i < types.length; i++) {
    const res = await handleGatehavenRoutes(
      new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/events`, {
        method: "POST",
        headers: { ...authHeaders(data.token), "Content-Type": "application/json" },
        body: JSON.stringify({ type: types[i], data: { n: i } }),
      }),
      env,
    );
    assert.equal(res!.status, 201, `event ${types[i]} must be 201`);
    const created = (await res!.json()) as any;
    assert.equal(created.seq, i + 1);
    assert.equal(typeof created.at, "number");
    const expected = await sha256Hex(
      canonicalJson({ seq: i + 1, type: types[i], data: { n: i }, at: created.at, prevHash: prev }),
    );
    assert.equal(created.entryHash, expected);
    hashes.push(created.entryHash);
    prev = created.entryHash;
  }

  const list = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/ledger`, {
      headers: authHeaders(data.token),
    }),
    env,
  );
  assert.equal(list!.status, 200);
  const body = (await list!.json()) as any;
  assert.equal(body.vaultId, data.vaultId);
  assert.equal(body.entries.length, 3);
  assert.deepEqual(
    body.entries.map((e: any) => e.seq),
    [1, 2, 3],
  );
  assert.deepEqual(
    body.entries.map((e: any) => e.type),
    [...types],
  );
  assert.deepEqual(
    body.entries.map((e: any) => e.entryHash),
    hashes,
  );
});

test("ledger: unknown event type is 400 and appends nothing", async () => {
  const env = makeFakeEnv();
  const { data } = await createAccount(env);
  const bad = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/events`, {
      method: "POST",
      headers: { ...authHeaders(data.token), "Content-Type": "application/json" },
      body: JSON.stringify({ type: "cheated", data: {} }),
    }),
    env,
  );
  assert.equal(bad!.status, 400);
  const list = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/ledger`, {
      headers: authHeaders(data.token),
    }),
    env,
  );
  const body = (await list!.json()) as any;
  assert.deepEqual(body.entries, []);
});

test("ledger: all five event types are accepted", async () => {
  const env = makeFakeEnv();
  const { data } = await createAccount(env);
  for (const type of [
    "quest_completed",
    "skill_learned",
    "dungeon_cleared",
    "item_soulbound",
    "level_gained",
  ]) {
    const res = await handleGatehavenRoutes(
      new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/events`, {
        method: "POST",
        headers: { ...authHeaders(data.token), "Content-Type": "application/json" },
        body: JSON.stringify({ type, data: { ok: true } }),
      }),
      env,
    );
    assert.equal(res!.status, 201, `event type ${type} must be accepted`);
  }
});

test("ledger: append-only — no update or delete route exists", async () => {
  const env = makeFakeEnv();
  const { data } = await createAccount(env);
  for (const method of ["PUT", "DELETE", "PATCH"]) {
    const res = await handleGatehavenRoutes(
      new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/events`, {
        method,
        headers: { ...authHeaders(data.token), "Content-Type": "application/json" },
        body: JSON.stringify({ type: "quest_completed", data: {} }),
      }),
      env,
    );
    assert.equal(res, null, `${method} /events must not be routed`);
  }
  const list = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/ledger`, {
      headers: authHeaders(data.token),
    }),
    env,
  );
  const body = (await list!.json()) as any;
  assert.deepEqual(body.entries, []);
});

test("ledger: fresh vault returns an empty entry list", async () => {
  const env = makeFakeEnv();
  const { data } = await createAccount(env);
  const res = await handleGatehavenRoutes(
    new Request(`https://gate/v1/gatehaven/vaults/${data.vaultId}/ledger`, {
      headers: authHeaders(data.token),
    }),
    env,
  );
  assert.equal(res!.status, 200);
  const body = (await res!.json()) as any;
  assert.deepEqual(body.entries, []);
});

// ---------------------------------------------------------------------------
// Canonical JSON helper
// ---------------------------------------------------------------------------
test("canonicalJson: sorts keys recursively and drops whitespace", () => {
  assert.equal(canonicalJson({ b: 1, a: { d: 4, c: 3 } }), '{"a":{"c":3,"d":4},"b":1}');
  assert.equal(canonicalJson([3, { z: 1, y: 2 }]), '[3,{"y":2,"z":1}]');
  assert.equal(canonicalJson("x"), '"x"');
  assert.equal(canonicalJson(null), "null");
});
