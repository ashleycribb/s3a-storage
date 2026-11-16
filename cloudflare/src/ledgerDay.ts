// Daily ledger rollover (UTC days). Pure logic over a minimal storage interface so it can be
// unit-tested with node --test without the Workers runtime.

export interface LedgerDayStorage {
  get<T>(key: string): Promise<T | undefined>;
  put<T>(key: string, value: T): Promise<void>;
}

export function utcDay(nowMs: number): number {
  return Math.floor(nowMs / 86400000);
}

/** Resets `ledger_spent_today` when a new UTC day has started. Returns true when it reset. */
export async function checkDailyReset(
  storage: LedgerDayStorage,
  nowMs: number,
  onReset: () => Promise<unknown>,
): Promise<boolean> {
  const currentDay = utcDay(nowMs);
  const lastResetDay = await storage.get<number>("ledger_last_reset_day");
  if (lastResetDay === undefined) {
    // First check ever (or state from before this was stored). We do not know which day the
    // existing spend belongs to, so it is kept; recording today makes tomorrow's reset happen.
    await storage.put("ledger_last_reset_day", currentDay);
    return false;
  }
  if (currentDay > lastResetDay) {
    await storage.put("ledger_last_reset_day", currentDay);
    await storage.put("ledger_spent_today", 0.0);
    await onReset();
    return true;
  }
  return false;
}
