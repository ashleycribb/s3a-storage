// Acceptance test (written before the fix): the daily budget must actually roll over at the UTC
// day boundary. Before the fix, ledger_last_reset_day was never stored, so spend accumulated
// forever and /health reported the ledger over cap (spent_today 11.3 of 5 on 2026-11-15).
import { test } from "node:test";
import assert from "node:assert/strict";
import { checkDailyReset, utcDay } from "../src/ledgerDay.ts";

class MapStorage {
  m = new Map<string, unknown>();
  async get<T>(k: string): Promise<T | undefined> { return this.m.get(k) as T | undefined; }
  async put<T>(k: string, v: T): Promise<void> { this.m.set(k, v); }
}

const DAY = 86400000;
const NOV14_NOON = Date.UTC(2026, 10, 14, 12, 0, 0);

test("fresh store: spend recorded on day 1 is reset on day 2", async () => {
  const s = new MapStorage();
  let resets = 0;
  // First budget check of the DO's life, then spend.
  await checkDailyReset(s, NOV14_NOON, async () => { resets++; });
  await s.put("ledger_spent_today", 4.5);
  // Next UTC day.
  const didReset = await checkDailyReset(s, NOV14_NOON + DAY, async () => { resets++; });
  assert.equal(didReset, true, "a new UTC day must reset the daily spend");
  assert.equal(await s.get("ledger_spent_today"), 0);
  assert.equal(resets, 1, "the deferred queue is drained exactly once");
  assert.equal(await s.get("ledger_last_reset_day"), utcDay(NOV14_NOON + DAY));
});

test("legacy store without ledger_last_reset_day still resets on the following day", async () => {
  // Shape of the live DO today: spend accumulated, no reset day ever stored.
  const s = new MapStorage();
  await s.put("ledger_spent_today", 11.29783);
  const first = await checkDailyReset(s, NOV14_NOON, async () => {});
  assert.equal(first, false, "unknown history: no reset is invented on the first check");
  assert.equal(await s.get("ledger_spent_today"), 11.29783);
  assert.equal(await s.get("ledger_last_reset_day"), utcDay(NOV14_NOON), "the current day must be recorded");
  const next = await checkDailyReset(s, NOV14_NOON + DAY, async () => {});
  assert.equal(next, true);
  assert.equal(await s.get("ledger_spent_today"), 0);
});

test("same UTC day never resets; the boundary is UTC midnight", async () => {
  const s = new MapStorage();
  const lateNov14 = Date.UTC(2026, 10, 14, 23, 59, 59);
  await checkDailyReset(s, Date.UTC(2026, 10, 14, 0, 0, 0), async () => {});
  await s.put("ledger_spent_today", 2);
  assert.equal(await checkDailyReset(s, lateNov14, async () => {}), false);
  assert.equal(await s.get("ledger_spent_today"), 2);
  assert.equal(await checkDailyReset(s, lateNov14 + 1000, async () => {}), true);
});
