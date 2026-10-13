"use client";

import { BracketBoard, type ReassignTarget, type TeamSlotRef } from "@/components/BracketBoard";
import type { FinalsLikeSnapshot } from "@/lib/bracket-board";
import { Button, Card } from "@brakets/ui";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { SeedBracketPanel } from "./SeedBracketPanel";

type LobbyReadySide = {
  lobbyId: string;
  sideId: string;
  readyBy: string;
  readyAt: string;
};

type ReadyResponse = {
  ready?: LobbyReadySide[];
  byLobbyId?: Record<string, LobbyReadySide[]>;
};

export type StreamPresenceLink = {
  provider: string;
  channelId: string;
  status: string;
  title?: string | null;
  error?: string | null;
};

export type StreamPresencePlayer = {
  userId: string;
  handle: string;
  displayName: string;
  links: StreamPresenceLink[];
};

export type StreamPresenceSnapshot = {
  configured: boolean;
  players: StreamPresencePlayer[];
};

function groupReadyByLobby(rows: LobbyReadySide[]): Record<string, LobbyReadySide[]> {
  const byLobbyId: Record<string, LobbyReadySide[]> = {};
  for (const row of rows) {
    const list = byLobbyId[row.lobbyId] ?? [];
    list.push(row);
    byLobbyId[row.lobbyId] = list;
  }
  return byLobbyId;
}

function readMatchParam(): string | null {
  if (typeof window === "undefined") return null;
  try {
    return new URLSearchParams(window.location.search).get("match");
  } catch {
    return null;
  }
}

function replaceMatchParam(matchId: string | null) {
  if (typeof window === "undefined") return;
  try {
    const url = new URL(window.location.href);
    if (matchId) url.searchParams.set("match", matchId);
    else url.searchParams.delete("match");
    const next = `${url.pathname}${url.search}${url.hash}`;
    const cur = `${window.location.pathname}${window.location.search}${window.location.hash}`;
    if (next === cur) return;
    window.history.replaceState(window.history.state, "", next);
  } catch {
    /* ignore malformed URL edge cases */
  }
}

type UndoReseat = { from: TeamSlotRef; to: ReassignTarget };

/** Invert a successful reseat so POST /reassign can undo it (one level). */
function invertReassign(from: TeamSlotRef, to: ReassignTarget, priorSlotIndex: number): UndoReseat {
  if (typeof to.teamId === "string" && to.teamId) {
    // Swap: moved team is now at to.lobbyId; counterpart is at from.lobbyId.
    return {
      from: { lobbyId: to.lobbyId, teamId: from.teamId },
      to: { lobbyId: from.lobbyId, teamId: to.teamId },
    };
  }
  // Empty-slot move: return team to prior lobby at its previous dense index.
  return {
    from: { lobbyId: to.lobbyId, teamId: from.teamId },
    to: { lobbyId: from.lobbyId, slotIndex: priorSlotIndex },
  };
}

function findTeamSlotIndex(
  snap: FinalsLikeSnapshot | null,
  lobbyId: string,
  teamId: string,
): number {
  if (!snap?.lobbies) return 0;
  const lobby = snap.lobbies.find((l) => l.id === lobbyId);
  if (!lobby) return 0;
  const idx = lobby.teamIds.indexOf(teamId);
  return idx >= 0 ? idx : lobby.teamIds.length;
}

export function BracketBoardClient({
  tournamentId,
  tournamentName,
  tournamentTitle,
  mode = "organize",
  embed = false,
}: {
  tournamentId: string;
  tournamentName: string;
  /** Game slug — drives Finals-only Opening Cashout preview labels. */
  tournamentTitle?: string;
  mode?: "organize" | "share";
  /** Share-only: OBS / stream-friendly chrome-stripped view (`?embed=1`). */
  embed?: boolean;
}) {
  const isShare = mode === "share";
  const isEmbed = isShare && embed;
  const [snapshot, setSnapshot] = useState<FinalsLikeSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedMatchId, setSelectedMatchId] = useState<string | null>(null);
  const [matchParamReady, setMatchParamReady] = useState(false);
  const [reassigning, setReassigning] = useState(false);
  const [reassignError, setReassignError] = useState<string | null>(null);
  const [undoReseat, setUndoReseat] = useState<UndoReseat | null>(null);
  /** Seq from the reassign response that created undoReseat — clear undo if board.seq drifts. */
  const undoSeqRef = useRef<number | null>(null);
  const [readyByLobbyId, setReadyByLobbyId] = useState<Record<string, LobbyReadySide[]>>({});
  const [streamPresence, setStreamPresence] = useState<StreamPresenceSnapshot | null>(null);
  const [autoFitKey, setAutoFitKey] = useState(0);
  const [seedNotice, setSeedNotice] = useState<string | null>(null);
  const [lastUpdatedAt, setLastUpdatedAt] = useState<number | null>(null);
  const [refreshFailed, setRefreshFailed] = useState(false);
  const snapshotRef = useRef<FinalsLikeSnapshot | null>(null);
  /** Organize: Fit once per mount when a seeded board first appears — not on soft polls. */
  const didAutoFitRef = useRef(false);

  useEffect(() => {
    snapshotRef.current = snapshot;
  }, [snapshot]);

  // Deep-link ?match= after mount (avoid SSR/client hydration mismatch).
  useEffect(() => {
    const t = window.setTimeout(() => {
      const fromUrl = readMatchParam();
      if (fromUrl) setSelectedMatchId(fromUrl);
      setMatchParamReady(true);
    }, 0);
    return () => window.clearTimeout(t);
  }, []);

  const fetchReady = useCallback(async () => {
    if (isShare) return;
    try {
      const res = await fetch(`/api/tournaments/${tournamentId}/ready`, { cache: "no-store" });
      if (!res.ok) return;
      const data = (await res.json().catch(() => ({}))) as ReadyResponse;
      if (data.byLobbyId) {
        setReadyByLobbyId(data.byLobbyId);
        return;
      }
      setReadyByLobbyId(groupReadyByLobby(data.ready ?? []));
    } catch {
      /* ready is best-effort beside bracket */
    }
  }, [tournamentId, isShare]);

  /** Soft stream presence — never blocks board refresh. Skip on share. */
  const fetchStreamPresence = useCallback(async () => {
    if (isShare) return;
    try {
      const res = await fetch(`/api/tournaments/${tournamentId}/stream-presence`, {
        cache: "no-store",
      });
      if (!res.ok) {
        return;
      }
      const data = (await res.json().catch(() => ({}))) as {
        configured?: boolean;
        players?: StreamPresencePlayer[];
      };
      setStreamPresence({
        configured: Boolean(data.configured),
        players: Array.isArray(data.players) ? data.players : [],
      });
    } catch {
      /* soft-fail — board stays usable */
    }
  }, [tournamentId, isShare]);

  const refresh = useCallback(async () => {
    try {
      const res = await fetch(`/api/tournaments/${tournamentId}/bracket`, { cache: "no-store" });
      const data = (await res.json().catch(() => ({}))) as FinalsLikeSnapshot & { error?: string };
      if (!res.ok) {
        if (res.status === 404) {
          setSnapshot(null);
          setError(null);
          setRefreshFailed(false);
          setLastUpdatedAt(Date.now());
          return;
        }
        setRefreshFailed(true);
        if (!snapshotRef.current) {
          setError(data.error ?? `bracket_failed_${res.status}`);
        }
        return;
      }
      setSnapshot(data);
      setError(null);
      setRefreshFailed(false);
      setLastUpdatedAt(Date.now());
      await Promise.all([fetchReady(), fetchStreamPresence()]);
    } catch {
      setRefreshFailed(true);
      if (!snapshotRef.current) {
        setError("network_error");
      }
    } finally {
      setLoading(false);
    }
  }, [tournamentId, fetchReady, fetchStreamPresence]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      setLoading(true);
      try {
        const res = await fetch(`/api/tournaments/${tournamentId}/bracket`, { cache: "no-store" });
        const data = (await res.json().catch(() => ({}))) as FinalsLikeSnapshot & {
          error?: string;
        };
        if (cancelled) return;
        if (!res.ok) {
          if (res.status === 404) {
            setSnapshot(null);
            setError(null);
            setRefreshFailed(false);
            setLastUpdatedAt(Date.now());
            return;
          }
          setRefreshFailed(true);
          if (!snapshotRef.current) {
            setError(data.error ?? `bracket_failed_${res.status}`);
          }
          return;
        }
        setSnapshot(data);
        setError(null);
        setRefreshFailed(false);
        setLastUpdatedAt(Date.now());
        await Promise.all([fetchReady(), fetchStreamPresence()]);
      } catch {
        if (!cancelled) {
          setRefreshFailed(true);
          if (!snapshotRef.current) setError("network_error");
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [tournamentId, fetchReady, fetchStreamPresence]);

  useEffect(() => {
    if (!snapshot || !selectedMatchId) return;
    const exists = (snapshot.lobbies ?? []).some((lobby) => lobby.id === selectedMatchId);
    if (!exists) {
      const t = window.setTimeout(() => setSelectedMatchId(null), 0);
      return () => window.clearTimeout(t);
    }
  }, [snapshot, selectedMatchId]);

  useEffect(() => {
    if (!matchParamReady) return;
    replaceMatchParam(selectedMatchId);
  }, [selectedMatchId, matchParamReady]);

  const clearUndoReseat = useCallback(() => {
    setUndoReseat(null);
    undoSeqRef.current = null;
  }, []);

  useEffect(() => {
    if (!snapshot || undoReseat == null) return;
    const expected = undoSeqRef.current;
    if (expected == null) return;
    if (snapshot.seq !== expected) {
      clearUndoReseat();
    }
  }, [snapshot, undoReseat, clearUndoReseat]);

  const postReassign = useCallback(
    async (from: TeamSlotRef, to: ReassignTarget, opts?: { asUndo?: boolean }) => {
      if (isShare) return;
      setReassignError(null);
      setReassigning(true);
      const priorSlotIndex = findTeamSlotIndex(snapshot, from.lobbyId, from.teamId);
      try {
        const res = await fetch(`/api/tournaments/${tournamentId}/bracket/reassign`, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ from, to }),
        });
        const data = (await res.json().catch(() => ({}))) as FinalsLikeSnapshot & {
          error?: string;
        };
        if (!res.ok) {
          setReassignError(data.error ?? `reassign_failed_${res.status}`);
          if (opts?.asUndo) clearUndoReseat();
          return;
        }
        setSnapshot(data);
        setReassignError(null);
        if (opts?.asUndo) {
          clearUndoReseat();
        } else {
          const inverse = invertReassign(from, to, priorSlotIndex);
          setUndoReseat(inverse);
          undoSeqRef.current = typeof data.seq === "number" ? data.seq : null;
        }
        await refresh();
      } catch {
        setReassignError("network_error");
        if (opts?.asUndo) clearUndoReseat();
      } finally {
        setReassigning(false);
      }
    },
    [tournamentId, refresh, isShare, snapshot, clearUndoReseat],
  );

  const onReassign = useCallback(
    async (from: TeamSlotRef, to: ReassignTarget) => {
      await postReassign(from, to);
    },
    [postReassign],
  );

  const onUndoReseat = useCallback(async () => {
    if (isShare || !undoReseat || reassigning) return;
    await postReassign(undoReseat.from, undoReseat.to, { asUndo: true });
  }, [isShare, undoReseat, reassigning, postReassign]);

  const onSeeded = useCallback(
    (data: unknown) => {
      const snap = data as FinalsLikeSnapshot;
      setSnapshot(snap);
      setError(null);
      setRefreshFailed(false);
      setLastUpdatedAt(Date.now());
      setLoading(false);
      const lobbies = snap.lobbies ?? [];
      const firstLive = lobbies.find((l) => l.status === "live") ?? lobbies[0];
      if (firstLive?.id) {
        setSelectedMatchId(firstLive.id);
      }
      didAutoFitRef.current = true;
      setAutoFitKey((k) => k + 1);
      setSeedNotice("Bracket seeded — drag to reseat, report from strip.");
      window.setTimeout(() => setSeedNotice(null), 8000);
      void Promise.all([fetchReady(), fetchStreamPresence()]);
    },
    [fetchReady, fetchStreamPresence],
  );

  const pollMs = useMemo(() => {
    if (isEmbed) return 20_000;
    if (isShare) return 15_000;
    if (!snapshot) return 15_000;
    if (snapshot.status === "completed" || snapshot.championTeamId) return 0;
    const anyLive =
      snapshot.status === "live" ||
      (snapshot.lobbies ?? []).some((lobby) => lobby.status === "live");
    return anyLive ? 8_000 : 15_000;
  }, [snapshot, isShare, isEmbed]);

  useEffect(() => {
    if (!isEmbed || !snapshot) return;
    const t = window.setTimeout(() => setAutoFitKey((k) => k + 1), 60);
    return () => window.clearTimeout(t);
  }, [isEmbed, snapshot]);

  useEffect(() => {
    if (isShare || isEmbed) return;
    if (!snapshot) return;
    if (didAutoFitRef.current) return;
    didAutoFitRef.current = true;
    const t = window.setTimeout(() => setAutoFitKey((k) => k + 1), 60);
    return () => window.clearTimeout(t);
  }, [snapshot, isShare, isEmbed]);

  useEffect(() => {
    if (isShare) return;
    let clearT: number | undefined;
    const t = window.setTimeout(() => {
      try {
        const url = new URL(window.location.href);
        if (url.searchParams.get("fit") !== "1") return;
        url.searchParams.delete("fit");
        const next = `${url.pathname}${url.search}${url.hash}`;
        window.history.replaceState(window.history.state, "", next);
        setSeedNotice("Finals cup live — Fit applied. Reseat/report from the strip.");
        clearT = window.setTimeout(() => setSeedNotice(null), 8000);
      } catch {
        /* ignore malformed URL */
      }
    }, 0);
    return () => {
      window.clearTimeout(t);
      if (clearT != null) window.clearTimeout(clearT);
    };
  }, [isShare]);

  return (
    <section className={`desk-shell${isEmbed ? " desk-shell--embed" : ""}`}>
      {isEmbed ? null : (
        <header className="desk-header">
          <div className="font-display font-bold text-xl tracking-wider text-accent uppercase">
            BRAKETS
          </div>
          <div>
            <p className="hero__eyebrow">
              {isShare ? "Public bracket · snapshot" : "Organizer desk · Bracket canvas"}
            </p>
            <h1 className="auth-title">{isShare ? "Public bracket" : "Bracket board"}</h1>
            <p className="auth-lede">
              <strong>{tournamentName}</strong>
              {isShare
                ? " · read-only · Refresh · Standings · Printable"
                : " · pan/zoom board · TO drag-to-reseat"}
            </p>
          </div>
          {isShare ? null : (
            <div className="row" style={{ justifyContent: "flex-start" }}>
              <Link href={`/tournaments/${tournamentId}/finals`}>
                <Button type="button" variant="primary" size="sm">
                  Finals run desk
                </Button>
              </Link>
              <Link href={`/tournaments/${tournamentId}/desk`}>
                <Button type="button" variant="secondary" size="sm">
                  Walk-up
                </Button>
              </Link>
              <Link href="/bracket">
                <Button type="button" variant="ghost" size="sm">
                  Reset
                </Button>
              </Link>
            </div>
          )}
        </header>
      )}

      <Card className="desk-panel bb-embed-card">
        {seedNotice && !isShare ? (
          <p className="bb-seed-notice" role="status">
            {seedNotice}
          </p>
        ) : null}
        <BracketBoard
          data={snapshot}
          loading={loading}
          error={error}
          onRefresh={refresh}
          pollMs={pollMs}
          lastUpdatedAt={lastUpdatedAt}
          refreshFailed={refreshFailed}
          selectedMatchId={selectedMatchId}
          onSelectMatch={setSelectedMatchId}
          mode={mode}
          embed={isEmbed}
          editable={!isShare}
          onReassign={isShare ? undefined : onReassign}
          reassigning={reassigning}
          reassignError={reassignError}
          onUndoReseat={isShare ? undefined : onUndoReseat}
          canUndoReseat={!isShare && undoReseat != null && !reassigning}
          readyByLobbyId={isShare ? undefined : readyByLobbyId}
          streamPresence={isShare ? undefined : streamPresence}
          autoFitKey={autoFitKey}
          tournamentId={tournamentId}
          emptyHref={isShare ? "" : "/bracket"}
          emptyLabel={isShare ? "" : "Create tournament"}
          emptyPanel={
            isShare ? undefined : (
              <SeedBracketPanel
                tournamentId={tournamentId}
                tournamentName={tournamentName}
                tournamentTitle={tournamentTitle}
                onSeeded={onSeeded}
              />
            )
          }
          runDeskHref={isShare ? undefined : `/tournaments/${tournamentId}/finals`}
          walkUpHref={isShare ? undefined : `/tournaments/${tournamentId}/desk`}
          discordHref={isShare ? undefined : `/tournaments/${tournamentId}/discord`}
          widgetHref={isShare ? undefined : `/tournaments/${tournamentId}/widget`}
        />
      </Card>
    </section>
  );
}
