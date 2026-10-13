"use client";

import { Button } from "@brakets/ui";
import { FinalsOpeningPreview } from "@/components/FinalsOpeningPreview";
import { humanizeCreateError } from "@/lib/create-errors";
import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";

const TEAM_SLOTS = [1, 2, 3, 4, 5, 6, 7, 8] as const;

type WalkUpEntry = {
  user: { id: string; handle: string; displayName: string } | null;
};

function uniqueRosterNames(entries: WalkUpEntry[]): string[] {
  const names: string[] = [];
  const seen = new Set<string>();
  for (const entry of entries) {
    const label = (entry.user?.displayName || entry.user?.handle || "").trim();
    if (!label) continue;
    const key = label.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    names.push(label);
  }
  return names;
}

function moveItem<T>(list: T[], from: number, to: number): T[] {
  if (to < 0 || to >= list.length) return list;
  const next = list.slice();
  const [item] = next.splice(from, 1);
  if (item === undefined) return list;
  next.splice(to, 0, item);
  return next;
}

/** Fisher–Yates shuffle of all slots (including empties). */
function shuffleAll<T>(list: T[]): T[] {
  const next = list.slice();
  for (let i = next.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    const tmp = next[i]!;
    next[i] = next[j]!;
    next[j] = tmp;
  }
  return next;
}

/** Swap Opening A (slots 0–3) with Opening B (slots 4–7). */
function swapOpenings(list: string[]): string[] {
  const next = list.slice();
  for (let i = 0; i < 4; i++) {
    const tmp = next[i]!;
    next[i] = next[i + 4]!;
    next[i + 4] = tmp;
  }
  return next;
}

const DEMO_TEAM_NAMES = [
  "Alpha",
  "Bravo",
  "Charlie",
  "Delta",
  "Echo",
  "Foxtrot",
  "Golf",
  "Hotel",
] as const;

/**
 * Organize-only empty-board panel: seed an 8-team bracket.
 */
export function SeedBracketPanel({
  tournamentId,
  tournamentName,
  tournamentTitle,
  onSeeded,
}: {
  tournamentId: string;
  tournamentName?: string;
  /** Game slug — Opening Cashout labels only when `the-finals`. */
  tournamentTitle?: string;
  onSeeded: (snapshot: unknown) => void;
}) {
  const [names, setNames] = useState<string[]>(() => Array.from({ length: 8 }, () => ""));
  const [rosterCount, setRosterCount] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const pendingRef = useRef(false);
  const [rosterBusy, setRosterBusy] = useState(false);

  const applyRosterPrefill = useCallback((entries: WalkUpEntry[]) => {
    const unique = uniqueRosterNames(entries);
    setRosterCount(unique.length);
    setNames((prev) => {
      const next = prev.slice();
      for (let i = 0; i < 8; i++) {
        if (!next[i]?.trim() && unique[i]) next[i] = unique[i]!;
      }
      return next;
    });
  }, []);

  const fetchRosterPrefill = useCallback(async () => {
    setRosterBusy(true);
    try {
      const res = await fetch(`/api/tournaments/${tournamentId}/walk-up`, { cache: "no-store" });
      if (!res.ok) return;
      const data = (await res.json().catch(() => ({}))) as { entries?: WalkUpEntry[] };
      applyRosterPrefill(Array.isArray(data.entries) ? data.entries : []);
    } catch {
      /* soft-fail — empty inputs are fine */
    } finally {
      setRosterBusy(false);
    }
  }, [tournamentId, applyRosterPrefill]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const res = await fetch(`/api/tournaments/${tournamentId}/walk-up`, { cache: "no-store" });
        if (!res.ok || cancelled) return;
        const data = (await res.json().catch(() => ({}))) as { entries?: WalkUpEntry[] };
        if (cancelled) return;
        applyRosterPrefill(Array.isArray(data.entries) ? data.entries : []);
      } catch {
        /* soft-fail — empty inputs are fine */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [tournamentId, applyRosterPrefill]);

  function setSlot(index: number, value: string) {
    setNames((prev) => {
      const next = prev.slice();
      next[index] = value;
      return next;
    });
  }

  function moveSlot(index: number, direction: -1 | 1) {
    setNames((prev) => moveItem(prev, index, index + direction));
  }

  function shuffleSlots() {
    setNames((prev) => shuffleAll(prev));
  }

  function swapOpeningGroups() {
    setNames((prev) => swapOpenings(prev));
  }

  function fillDemoTeams() {
    setNames(DEMO_TEAM_NAMES.slice());
  }

  function clearAllTeams() {
    if (names.some((n) => n.trim().length > 0)) {
      const ok = window.confirm("Clear all 8 team names?");
      if (!ok) return;
    }
    setNames(Array.from({ length: 8 }, () => ""));
  }

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    if (pendingRef.current || pending) return;
    setError(null);
    const anyFilled = names.some((n) => n.trim().length > 0);
    if (anyFilled) {
      const ok = window.confirm("Seed 8 teams? This starts the live bracket.");
      if (!ok) return;
    }
    const teams = names.map((n) => n.trim()).filter(Boolean);
    if (teams.length !== 8) {
      setError(humanizeCreateError("eight_team_names_required"));
      return;
    }
    pendingRef.current = true;
    setPending(true);
    try {
      const res = await fetch(`/api/tournaments/${tournamentId}/bracket/seed`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ teams }),
      });
      const data = (await res.json().catch(() => ({}))) as {
        error?: string;
        issues?: Array<{ path?: (string | number)[]; message?: string }>;
        tournamentId?: string;
        lobbies?: unknown[];
      };
      if (!res.ok) {
        setError(humanizeCreateError(data.error ?? "seed_failed", data.issues));
        pendingRef.current = false;
        setPending(false);
        return;
      }
      onSeeded(data);
    } catch {
      setError(humanizeCreateError("network_error"));
      pendingRef.current = false;
      setPending(false);
    }
  }

  return (
    <form className="bb-seed-panel auth-form" onSubmit={(e) => void onSubmit(e)}>
      <h2 className="desk-section-title">
        Seed{tournamentName ? ` ${tournamentName}` : " this tournament"}
      </h2>
      <p className="journey-row__meta">
        Seed an 8-team bracket to run this cup. Roster has {rosterCount} player
        {rosterCount === 1 ? "" : "s"} — need 8 team names
        {rosterCount >= 8 ? " (prefilled from walk-up when slots were empty)." : "."} Reorder with
        Up/Down or Shuffle before seeding.
      </p>
      <div className="bb-seed-toolbar row">
        <Button
          type="button"
          variant="secondary"
          size="sm"
          disabled={rosterBusy || pending}
          onClick={() => void fetchRosterPrefill()}
        >
          {rosterBusy ? "Loading roster…" : "Prefill from roster"}
        </Button>
        <Button type="button" variant="ghost" size="sm" disabled={pending} onClick={shuffleSlots}>
          Shuffle
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={pending}
          onClick={swapOpeningGroups}
          title="Swap Opening A (teams 1–4) with Opening B (teams 5–8)"
        >
          Swap A↔B
        </Button>
        <Button type="button" variant="ghost" size="sm" disabled={pending} onClick={fillDemoTeams}>
          Fill demo teams
        </Button>
        <Button type="button" variant="ghost" size="sm" disabled={pending} onClick={clearAllTeams}>
          Clear
        </Button>
      </div>
      <div className="desk-fields">
        {TEAM_SLOTS.map((n, index) => (
          <div className="field bb-seed-row" key={n}>
            <label htmlFor={`seed-team-${n}`}>Team {n}</label>
            <div className="bb-seed-row__controls">
              <input
                id={`seed-team-${n}`}
                value={names[index] ?? ""}
                onChange={(e) => setSlot(index, e.target.value)}
                maxLength={64}
                placeholder={`Team ${n}`}
                autoComplete="off"
              />
              <span className="bb-seed-row__moves">
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  disabled={index === 0 || pending}
                  aria-label={`Move team ${n} up`}
                  onClick={() => moveSlot(index, -1)}
                >
                  Up
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  disabled={index === TEAM_SLOTS.length - 1 || pending}
                  aria-label={`Move team ${n} down`}
                  onClick={() => moveSlot(index, 1)}
                >
                  Down
                </Button>
              </span>
            </div>
          </div>
        ))}
      </div>
      <FinalsOpeningPreview
        names={names}
        action="seed"
        variant={tournamentTitle === "the-finals" ? "finals" : "neutral"}
      />
      {error ? <p className="form-error">{error}</p> : null}
      <div className="row" style={{ justifyContent: "flex-start" }}>
        <Button type="submit" variant="primary" size="sm" disabled={pending}>
          {pending ? "Seeding…" : "Seed 8-team bracket"}
        </Button>
      </div>
    </form>
  );
}
