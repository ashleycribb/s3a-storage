"use client";

import { Button, StatusChip, Tag } from "@brakets/ui";
import Link from "next/link";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent as ReactDragEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  type WheelEvent as ReactWheelEvent,
} from "react";
import {
  boardFromFinalsSnapshot,
  groupBoardColumns,
  type BoardMatch,
  type BoardSnapshot,
  type BoardTeam,
  type FinalsLikeSnapshot,
} from "@/lib/bracket-board";

export type TeamSlotRef = { lobbyId: string; teamId: string };

/** Drop target: occupied seat (swap) or empty capacity slot (move). */
export type ReassignTarget =
  | { lobbyId: string; teamId: string; slotIndex?: undefined }
  | { lobbyId: string; slotIndex: number; teamId?: undefined };

export type LobbyReadySide = {
  lobbyId: string;
  sideId: string;
  readyBy: string;
  readyAt: string;
};

type ReportAuditEvent = {
  id: string;
  lobbyId: string;
  reportedBy: string;
  role: string;
  status: string;
  createdAt: string;
  notes?: string;
};

type ReportAuditResponse = {
  events?: ReportAuditEvent[];
  conflictLobbyIds?: string[];
  evidenceCountsByLobbyId?: Record<string, number>;
};

/** Desk-auth evidence list row (+ viewPath proxy). Soft-fail when missing. */
type EvidenceItem = {
  id: string;
  lobbyId: string;
  contentType: string;
  viewPath: string;
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

export type BracketBoardProps = {
  data?: BoardSnapshot | FinalsLikeSnapshot | null;
  loading?: boolean;
  error?: string | null;
  onRefresh?: () => void | Promise<void>;
  /** Soft poll interval (ms). Phase C lite uses this for live refresh. */
  pollMs?: number;
  selectedMatchId?: string | null;
  /** Pass null to clear the selected-match strip. */
  onSelectMatch?: (matchId: string | null) => void;
  /** Organizer/TO mode: enable HTML5 drag-to-reseat on team slots. */
  editable?: boolean;
  /** Called on drop: swap (to.teamId) or empty-slot move (to.slotIndex). */
  onReassign?: (from: TeamSlotRef, to: ReassignTarget) => void | Promise<void>;
  reassignError?: string | null;
  reassigning?: boolean;
  /** One-level undo for the last successful reseat (organize). */
  onUndoReseat?: () => void | Promise<void>;
  canUndoReseat?: boolean;
  compact?: boolean;
  className?: string;
  emptyHref?: string;
  emptyLabel?: string;
  /** Organize-only: seed-this-tournament panel rendered in empty state. */
  emptyPanel?: ReactNode;
  /** Optional CTA from selected-match detail → Finals run desk. */
  runDeskHref?: string;
  /** Optional CTA from selected-match detail → Walk-up desk (also empty-state secondary). */
  walkUpHref?: string;
  /** Optional CTA from selected-match detail → Discord desk. */
  discordHref?: string;
  /** Optional CTA → Game Bar / widget. */
  widgetHref?: string;
  /** Tournament id for cup-smoke localStorage (organize; works before seed). */
  tournamentId?: string;
  /** Epoch ms of last successful bracket refresh (incl. empty 404). */
  lastUpdatedAt?: number | null;
  /** Soft poll/network failure — non-blocking; board stays if loaded. */
  refreshFailed?: boolean;
  /** GB-ready rows grouped by lobbyId (soft-fail upstream). */
  readyByLobbyId?: Record<string, LobbyReadySide[]>;
  /**
   * Soft stream presence (organize only). Strip/summary — never mapped onto roster nodes.
   * Missing / unconfigured / empty → render nothing.
   */
  streamPresence?: StreamPresenceSnapshot | null;
  /**
   * organize (default): TO desk — audit, resolve, share link, Log tab.
   * share: public Score7-ish snapshot — no audit/report/resolve/Log.
   */
  mode?: "organize" | "share";
  /**
   * Share embed (`?embed=1`): strip chrome for OBS Browser Source.
   * Auto Fit on load + resize; tabs/toolbar noise hidden.
   */
  embed?: boolean;
  /**
   * Increment to programmatically run Fit once (post-seed landing).
   * Ignored when undefined/0; each new positive value triggers fitView.
   */
  autoFitKey?: number;
};

/** Relative label for last successful board refresh. */
function formatUpdatedAgo(lastUpdatedAt: number, now: number): string {
  const sec = Math.max(0, Math.floor((now - lastUpdatedAt) / 1000));
  if (sec < 5) return "Updated just now";
  if (sec < 60) return `Updated ${sec}s ago`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `Updated ${min}m ago`;
  return `Updated ${Math.floor(min / 60)}h ago`;
}

type CupSmokeChecks = {
  walkUp: boolean;
  share: boolean;
  discord: boolean;
  widget: boolean;
  report: boolean;
};

const CUP_SMOKE_DEFAULTS: CupSmokeChecks = {
  walkUp: false,
  share: false,
  discord: false,
  widget: false,
  report: false,
};

function cupSmokeStorageKey(tournamentId: string) {
  return `brakets_cup_smoke_${tournamentId}`;
}

function readCupSmoke(tournamentId: string): { open: boolean; checks: CupSmokeChecks } {
  if (typeof window === "undefined") return { open: true, checks: { ...CUP_SMOKE_DEFAULTS } };
  try {
    const raw = localStorage.getItem(cupSmokeStorageKey(tournamentId));
    if (!raw) return { open: true, checks: { ...CUP_SMOKE_DEFAULTS } };
    const parsed = JSON.parse(raw) as Partial<CupSmokeChecks & { open?: boolean }>;
    return {
      open: parsed.open !== false,
      checks: {
        walkUp: Boolean(parsed.walkUp),
        share: Boolean(parsed.share),
        discord: Boolean(parsed.discord),
        widget: Boolean(parsed.widget),
        report: Boolean(parsed.report),
      },
    };
  } catch {
    return { open: true, checks: { ...CUP_SMOKE_DEFAULTS } };
  }
}

function writeCupSmoke(tournamentId: string, open: boolean, checks: CupSmokeChecks) {
  if (typeof window === "undefined") return;
  try {
    localStorage.setItem(cupSmokeStorageKey(tournamentId), JSON.stringify({ open, ...checks }));
  } catch {
    /* ignore quota / private mode */
  }
}

/** Soft-tick one cup-smoke checklist key (localStorage + caller should bump React state). */
function softTickCupSmokeKey(
  tournamentId: string | null | undefined,
  key: keyof CupSmokeChecks,
): void {
  if (!tournamentId) return;
  try {
    const stored = readCupSmoke(tournamentId);
    if (stored.checks[key]) return;
    writeCupSmoke(tournamentId, stored.open, { ...stored.checks, [key]: true });
  } catch {
    /* soft-fail — checklist helpers / private mode */
  }
}

function isFinalsLike(data: BoardSnapshot | FinalsLikeSnapshot): data is FinalsLikeSnapshot {
  return Array.isArray((data as FinalsLikeSnapshot).lobbies);
}

function normalize(
  data: BoardSnapshot | FinalsLikeSnapshot | null | undefined,
): BoardSnapshot | null {
  if (!data) return null;
  return isFinalsLike(data) ? boardFromFinalsSnapshot(data) : data;
}

function teamMap(teams: BoardTeam[]): Map<string, BoardTeam> {
  const map = new Map<string, BoardTeam>();
  for (const team of teams) map.set(team.id, team);
  return map;
}

/** Honest board phase label — never show LIVE on pending/preview. */
function boardPhase(board: BoardSnapshot): { label: string; kind: "live" | "upcoming" | "final" } {
  if (board.status === "completed" || board.championTeamId) {
    return { label: "Complete", kind: "final" };
  }
  const anyLive = board.matches.some((m) => m.status === "live");
  if (anyLive || board.status === "live") {
    return { label: "Live", kind: "live" };
  }
  const seated = board.matches.some((m) => m.teamIds.length > 0);
  if (!seated) return { label: "Preview", kind: "upcoming" };
  return { label: "Upcoming", kind: "upcoming" };
}

function matchPhaseLabel(
  match: BoardMatch,
  inConflict = false,
): {
  label: string;
  kind: "live" | "upcoming" | "ready" | "final";
} {
  if (inConflict || match.status === "disputed" || match.status === "conflict") {
    return { label: "Disputed", kind: "ready" };
  }
  if (match.status === "live") return { label: "Live", kind: "live" };
  if (match.status === "final" || match.status === "verified")
    return { label: "Complete", kind: "final" };
  if (match.status === "reported") {
    return { label: "Reported", kind: "ready" };
  }
  if (match.teamIds.length === 0) return { label: "Preview", kind: "upcoming" };
  return { label: "Upcoming", kind: "upcoming" };
}

function matchCapacity(match: BoardMatch): number {
  if (match.kind === "final_round") return 3;
  if (match.kind === "cashout") return 4;
  return Math.max(2, match.teamIds.length);
}

/** Cheap TO-facing copy for common bracket load codes. */
function humanizeBoardError(error: string): string {
  if (error === "network_error") {
    return "Network error — check your connection and try again.";
  }
  if (error === "forbidden" || error === "bracket_failed_403") {
    return "You don't have access to this bracket.";
  }
  if (error === "not_found" || error === "bracket_not_found" || error === "bracket_failed_404") {
    return "No bracket found for this tournament yet.";
  }
  if (error.startsWith("bracket_failed_")) {
    return `Couldn't load bracket (HTTP ${error.replace("bracket_failed_", "")}).`;
  }
  return error.replace(/_/g, " ");
}

function lobbyReassignable(status: string): boolean {
  return status === "scheduled" || status === "live";
}

function moveItem<T>(list: T[], from: number, to: number): T[] {
  if (to < 0 || to >= list.length) return list;
  const next = list.slice();
  const [item] = next.splice(from, 1);
  if (item === undefined) return list;
  next.splice(to, 0, item);
  return next;
}

function humanizeReportError(error: string): string {
  if (error === "network_error") {
    return "Network error — check your connection and try again.";
  }
  if (error === "forbidden" || error === "unauthorized") {
    return "You don't have permission to report standings.";
  }
  if (error === "lobby_not_found" || error === "not_found") {
    return "That lobby isn't on this bracket anymore.";
  }
  if (error === "invalid_standings" || error === "standings_mismatch") {
    return "Standings don't match the seated teams — refresh and try again.";
  }
  if (error.startsWith("report_failed_")) {
    return `Couldn't submit report (HTTP ${error.replace("report_failed_", "")}).`;
  }
  return error.replace(/_/g, " ");
}

function reportStatusNotice(status: string | undefined): string {
  if (status === "duplicate") {
    return "Duplicate report recorded (same standings) — bracket unchanged.";
  }
  if (status === "corroborated") {
    return "Corroborated by a second reporter — bracket unchanged.";
  }
  if (status === "conflict") {
    return "Conflict soft-flagged — new standings accepted; check run desk audit.";
  }
  if (status === "accepted") {
    return "Standings reported.";
  }
  return "Standings reported.";
}

/** Soft-poll cadence from board phase — Complete stops; Live is snappier. */

function auditStatusKind(status: string): "live" | "upcoming" | "ready" | "final" {
  if (status === "conflict") return "live";
  if (status === "accepted" || status === "corroborated") return "ready";
  if (status === "resolved" || status === "rejected") return "final";
  return "upcoming";
}

function truncateId(id: string, n = 8): string {
  if (id.length <= n) return id;
  return `${id.slice(0, n)}…`;
}

/** 1 → 1st, 2 → 2nd, 3 → 3rd, 4 → 4th… */
function ordinal(n: number): string {
  const v = n % 100;
  if (v >= 11 && v <= 13) return `${n}th`;
  switch (n % 10) {
    case 1:
      return `${n}st`;
    case 2:
      return `${n}nd`;
    case 3:
      return `${n}rd`;
    default:
      return `${n}th`;
  }
}

function truncateName(name: string, max = 18): string {
  if (name.length <= max) return name;
  return `${name.slice(0, Math.max(1, max - 1))}…`;
}

/** Kind-aware hint under the report strip title. */
function reportStripHint(kind: string | undefined, teamCount: number): string | null {
  if (kind === "cashout" || teamCount === 4) {
    return "Opening/Cashout: rank best → worst, then submit.";
  }
  if (kind === "final_round" || teamCount === 3) {
    return "Final: rank best → worst.";
  }
  return null;
}

function formatAuditTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function adaptivePollMs(board: BoardSnapshot | null, fallback?: number): number {
  if (!board) return fallback && fallback > 0 ? fallback : 0;
  const phase = boardPhase(board);
  if (phase.kind === "final") return 0;
  if (phase.kind === "live") return 8_000;
  return fallback && fallback > 0 ? fallback : 15_000;
}

function liveStreamHandles(presence: StreamPresenceSnapshot | null | undefined): Array<{
  key: string;
  handle: string;
  provider: string;
  channelId: string;
}> {
  if (!presence?.configured || !presence.players?.length) return [];
  const out: Array<{ key: string; handle: string; provider: string; channelId: string }> = [];
  for (const player of presence.players) {
    for (const link of player.links ?? []) {
      if (link.status !== "live") continue;
      const provider = String(link.provider ?? "").toLowerCase();
      if (provider !== "twitch" && provider !== "youtube") continue;
      out.push({
        key: `${player.userId}:${provider}:${link.channelId}`,
        handle: player.handle || player.displayName || link.channelId,
        provider,
        channelId: link.channelId,
      });
      if (out.length >= 6) return out;
    }
  }
  return out;
}

function nextLiveMatchId(matches: BoardMatch[], fromId: string): string | null {
  const idx = matches.findIndex((m) => m.id === fromId);
  if (idx < 0) {
    return matches.find((m) => m.status === "live")?.id ?? null;
  }
  for (let i = idx + 1; i < matches.length; i++) {
    if (matches[i]!.status === "live") return matches[i]!.id;
  }
  for (let i = 0; i < idx; i++) {
    if (matches[i]!.status === "live") return matches[i]!.id;
  }
  return null;
}

function cycleMatchId(
  matches: BoardMatch[],
  selectedId: string | null,
  direction: 1 | -1,
): string | null {
  if (matches.length === 0) return null;
  const live = matches.filter((m) => m.status === "live");
  const pool = live.length > 0 ? live : matches;
  if (pool.length === 0) return null;
  const ids = pool.map((m) => m.id);
  const cur = selectedId ? ids.indexOf(selectedId) : -1;
  if (cur < 0) return direction > 0 ? ids[0]! : ids[ids.length - 1]!;
  const next = (cur + direction + ids.length) % ids.length;
  return ids[next]!;
}

function countAllReadyLobbies(
  matches: BoardMatch[],
  readyByLobbyId?: Record<string, LobbyReadySide[]>,
): number {
  if (!readyByLobbyId) return 0;
  let n = 0;
  for (const m of matches) {
    const rows = readyByLobbyId[m.id];
    if (!rows?.length || m.teamIds.length === 0) continue;
    const readySet = new Set(rows.map((r) => r.sideId));
    if (m.teamIds.every((id) => readySet.has(id))) n += 1;
  }
  return n;
}

type DragPayload = { lobbyId: string; teamId: string };

function MatchNode({
  match,
  teams,
  selected,
  onSelect,
  editable,
  dragOverKey,
  onDragStartSlot,
  onDragOverSlot,
  onDragLeaveSlot,
  onDropSlot,
  reassigning,
  flash,
  readyRows,
  inConflict,
  evidenceCount = 0,
}: {
  match: BoardMatch;
  teams: Map<string, BoardTeam>;
  selected: boolean;
  onSelect?: (id: string) => void;
  editable?: boolean;
  dragOverKey: string | null;
  onDragStartSlot: (from: DragPayload) => void;
  onDragOverSlot: (key: string) => void;
  onDragLeaveSlot: () => void;
  onDropSlot: (to: ReassignTarget) => void;
  reassigning?: boolean;
  flash?: boolean;
  readyRows?: LobbyReadySide[];
  inConflict?: boolean;
  evidenceCount?: number;
}) {
  const phase = matchPhaseLabel(match, Boolean(inConflict));
  const live = match.status === "live";
  const canEdit = Boolean(editable && lobbyReassignable(match.status) && !reassigning);
  const readySet = new Set((readyRows ?? []).map((row) => row.sideId));
  const seatedCount = match.teamIds.length;
  const readyCount = match.teamIds.filter((id) => readySet.has(id)).length;
  const showReady = (readyRows?.length ?? 0) > 0 && seatedCount > 0;
  const allReady = showReady && readyCount === seatedCount;
  // When standings exist the lobby is reported/final — seats are locked for DnD.
  // Display still prefers standings order when present.
  const order = match.standings && match.standings.length > 0 ? match.standings : match.teamIds;
  const winnerId = match.standings?.[0];
  // Compact: Finals cashout shows up to 4; final_round 3; classic consumers 2.
  const minSlots = match.kind === "final_round" ? 3 : match.kind === "cashout" ? 4 : 2;
  const slots = Math.max(order.length, minSlots);
  const rows: Array<string | null> = [];
  for (let i = 0; i < slots; i++) rows.push(order[i] ?? null);

  return (
    <article
      className={`bb-node${selected ? " bb-node--selected" : ""}${live ? " bb-node--live" : ""}${inConflict ? " bb-node--conflict" : ""}${phase.label === "Complete" ? " bb-node--complete" : ""}${flash ? " bb-node--flash" : ""}`}
      data-match-id={match.id}
      data-status={match.status}
      onClick={() => onSelect?.(match.id)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect?.(match.id);
        }
      }}
      role="button"
      tabIndex={0}
      aria-label={`${match.label} · ${phase.label}`}
    >
      <header className="bb-node__head">
        <span className="bb-node__title">{match.label}</span>
        <span className="bb-node__chips">
          {showReady ? (
            <StatusChip
              kind={allReady ? "ready" : "upcoming"}
              label={allReady ? "All ready" : `Ready ${readyCount}/${seatedCount}`}
            />
          ) : null}
          {evidenceCount > 0 ? (
            <span
              className="bb-node__evidence"
              title={`${evidenceCount} evidence attachment${evidenceCount === 1 ? "" : "s"}`}
              aria-label={`${evidenceCount} evidence`}
            >
              📎{evidenceCount}
            </span>
          ) : null}
          <StatusChip kind={phase.kind} label={phase.label} />
        </span>
      </header>
      <ul className="bb-node__teams">
        {rows.map((id, index) => {
          if (!id) {
            const emptyKey = `${match.id}:empty:${index}`;
            const isEmptyTarget = canEdit && dragOverKey === emptyKey;
            return (
              <li
                key={`empty-${index}`}
                className={`bb-team bb-team--empty${canEdit ? " bb-team--droppable" : ""}${isEmptyTarget ? " bb-team--drop-target" : ""}`}
                data-lobby-id={match.id}
                data-slot-index={index}
                onDragOver={(e: ReactDragEvent<HTMLLIElement>) => {
                  if (!canEdit) return;
                  e.preventDefault();
                  e.stopPropagation();
                  e.dataTransfer.dropEffect = "move";
                  onDragOverSlot(emptyKey);
                }}
                onDragLeave={() => {
                  onDragLeaveSlot();
                }}
                onDrop={(e: ReactDragEvent<HTMLLIElement>) => {
                  if (!canEdit) return;
                  e.preventDefault();
                  e.stopPropagation();
                  onDropSlot({ lobbyId: match.id, slotIndex: index });
                  onDragLeaveSlot();
                }}
              >
                <span className="bb-team__seed" aria-hidden="true">
                  —
                </span>
                <span className="bb-team__name">{canEdit ? "Drop here" : "TBD"}</span>
                <span className="bb-team__score">–</span>
              </li>
            );
          }
          const team = teams.get(id);
          const isWinner = winnerId === id && Boolean(match.standings?.length);
          const place = match.standings?.length ? index + 1 : null;
          const slotKey = `${match.id}:${id}`;
          const isDropTarget = dragOverKey === slotKey;
          return (
            <li
              key={id}
              className={`bb-team${isWinner ? " bb-team--winner" : ""}${canEdit ? " bb-team--draggable" : ""}${isDropTarget ? " bb-team--drop-target" : ""}`}
              data-team-id={id}
              data-lobby-id={match.id}
              draggable={canEdit}
              onDragStart={(e: ReactDragEvent<HTMLLIElement>) => {
                if (!canEdit) return;
                e.stopPropagation();
                const payload: DragPayload = { lobbyId: match.id, teamId: id };
                onDragStartSlot(payload);
                e.dataTransfer.setData("application/x-brakets-team", JSON.stringify(payload));
                e.dataTransfer.setData("text/plain", `${match.id}:${id}`);
                e.dataTransfer.effectAllowed = "move";
                (e.currentTarget as HTMLElement).classList.add("bb-team--dragging");
              }}
              onDragEnd={(e: ReactDragEvent<HTMLLIElement>) => {
                (e.currentTarget as HTMLElement).classList.remove("bb-team--dragging");
                onDragLeaveSlot();
              }}
              onDragOver={(e: ReactDragEvent<HTMLLIElement>) => {
                if (!canEdit) return;
                e.preventDefault();
                e.stopPropagation();
                e.dataTransfer.dropEffect = "move";
                onDragOverSlot(slotKey);
              }}
              onDragLeave={() => {
                onDragLeaveSlot();
              }}
              onDrop={(e: ReactDragEvent<HTMLLIElement>) => {
                if (!canEdit) return;
                e.preventDefault();
                e.stopPropagation();
                onDropSlot({ lobbyId: match.id, teamId: id });
                onDragLeaveSlot();
              }}
            >
              {canEdit ? (
                <span className="bb-team__handle" title="Drag to reseat" aria-hidden="true">
                  ⋮⋮
                </span>
              ) : (
                <span className="bb-team__seed">{team?.seed != null ? team.seed : "·"}</span>
              )}
              <span className="bb-team__name">
                {canEdit && team?.seed != null ? (
                  <span className="bb-team__seed-inline">#{team.seed}</span>
                ) : null}
                {team?.name ?? id}
              </span>
              <span className={`bb-team__score${isWinner ? " bb-team__score--win" : ""}`}>
                {place != null ? place : "–"}
              </span>
            </li>
          );
        })}
      </ul>
      {showReady ? (
        <footer className="bb-node__ready" aria-label="Lobby ready state">
          {allReady ? (
            <span className="bb-node__ready-all">All ready</span>
          ) : (
            <span className="bb-node__ready-dots">
              {match.teamIds.map((id) => (
                <span
                  key={id}
                  className={`bb-ready-dot${readySet.has(id) ? " bb-ready-dot--on" : ""}`}
                  title={readySet.has(id) ? "Ready" : "Not ready"}
                  aria-hidden="true"
                />
              ))}
              <span className="bb-node__ready-count">
                Ready {readyCount}/{seatedCount}
              </span>
            </span>
          )}
        </footer>
      ) : null}
    </article>
  );
}

/** Challonge-style elbow connectors between adjacent round columns. */
function ConnectorLayer({ columns }: { columns: ReturnType<typeof groupBoardColumns> }) {
  const paths = useMemo(() => {
    const segs: string[] = [];
    const colW = 268;
    const gap = 72;
    const headerH = 36;
    const nodeH = 148;
    const nodeGap = 20;
    for (let c = 0; c < columns.length - 1; c++) {
      const left = columns[c]!;
      const right = columns[c + 1]!;
      const x1 = (c + 1) * colW + c * gap;
      const x2 = x1 + gap;
      const mid = x1 + gap / 2;
      for (let ri = 0; ri < right.matches.length; ri++) {
        const parents = left.matches;
        if (parents.length === 0) continue;
        // Pair-wise when 2:1, else fan from nearest parents.
        const ratio = parents.length / Math.max(right.matches.length, 1);
        const pStart = Math.floor(ri * ratio);
        const pEnd = Math.min(parents.length - 1, Math.floor((ri + 1) * ratio) - 1);
        const destY =
          headerH + ri * (nodeH + nodeGap) + nodeH / 2 + (right.matches.length === 1 ? 24 : 0);
        for (let pi = Math.max(0, pStart); pi <= Math.max(pStart, pEnd); pi++) {
          const srcY =
            headerH + pi * (nodeH + nodeGap) + nodeH / 2 + (left.matches.length === 1 ? 24 : 0);
          segs.push(`M ${x1} ${srcY} H ${mid} V ${destY} H ${x2}`);
        }
      }
    }
    return segs;
  }, [columns]);

  if (paths.length === 0) return null;
  const width = columns.length * 268 + Math.max(0, columns.length - 1) * 72;
  const height = Math.max(...columns.map((col) => 36 + col.matches.length * (148 + 20) + 40), 200);

  return (
    <svg className="bb-connectors" width={width} height={height} aria-hidden="true">
      {paths.map((d, i) => (
        <path key={i} d={d} fill="none" stroke="var(--border-2)" strokeWidth="2" />
      ))}
    </svg>
  );
}

export function BracketBoard({
  data,
  loading = false,
  error = null,
  onRefresh,
  pollMs,
  selectedMatchId = null,
  onSelectMatch,
  editable = false,
  onReassign,
  reassignError = null,
  reassigning = false,
  onUndoReseat,
  canUndoReseat = false,
  compact = false,
  className,
  emptyHref = "/tournaments/finals/new",
  emptyLabel = "Create Finals cup",
  emptyPanel,
  runDeskHref,
  walkUpHref,
  discordHref,
  widgetHref,
  tournamentId: tournamentIdProp,
  lastUpdatedAt = null,
  refreshFailed = false,
  readyByLobbyId,
  streamPresence = null,
  mode = "organize",
  embed = false,
  autoFitKey,
}: BracketBoardProps) {
  const isShare = mode === "share";
  const isOrganize = !isShare;
  const isEmbed = isShare && embed;
  const board = useMemo(() => normalize(data), [data]);
  const columns = useMemo(() => (board ? groupBoardColumns(board) : []), [board]);
  const teams = useMemo(() => teamMap(board?.teams ?? []), [board]);
  const phase = board ? boardPhase(board) : null;

  const viewportRef = useRef<HTMLDivElement>(null);
  const shellRef = useRef<HTMLElement>(null);
  const [transform, setTransform] = useState({ x: 20, y: 16, k: 1 });
  const dragRef = useRef<{
    pointerId: number;
    startX: number;
    startY: number;
    origX: number;
    origY: number;
  } | null>(null);
  const dragPayloadRef = useRef<DragPayload | null>(null);
  const [panning, setPanning] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [tab, setTab] = useState<"bracket" | "standings" | "announcements" | "log">("bracket");
  const lastSeqRef = useRef<number | null>(null);
  const [pulse, setPulse] = useState(false);
  const [dragOverKey, setDragOverKey] = useState<string | null>(null);
  const prevStatusesRef = useRef<Map<string, string> | null>(null);
  const [flashIds, setFlashIds] = useState<Set<string>>(() => new Set());
  const [updateBanner, setUpdateBanner] = useState<string | null>(null);
  const [reportOverrides, setReportOverrides] = useState<Record<string, string[]>>({});
  /** Organize-only: hide champion banner for this session. */
  const [championBannerDismissed, setChampionBannerDismissed] = useState(false);
  const [reportBusy, setReportBusy] = useState(false);
  const [reportNotice, setReportNotice] = useState<string | null>(null);
  const [reportError, setReportError] = useState<string | null>(null);
  /** Only show strip report feedback for the match that produced it. */
  const [reportFeedbackMatchId, setReportFeedbackMatchId] = useState<string | null>(null);
  const [auditEvents, setAuditEvents] = useState<ReportAuditEvent[]>([]);
  const [conflictLobbyIds, setConflictLobbyIds] = useState<string[]>([]);
  const [evidenceCountsByLobbyId, setEvidenceCountsByLobbyId] = useState<Record<string, number>>(
    {},
  );
  /** Full evidence rows for match-strip preview (organize only; soft-fail). */
  const [evidenceItems, setEvidenceItems] = useState<EvidenceItem[]>([]);
  const [lightboxSrc, setLightboxSrc] = useState<string | null>(null);
  const [keysOpen, setKeysOpen] = useState(false);
  const [auditError, setAuditError] = useState<string | null>(null);
  const [auditLoading, setAuditLoading] = useState(false);
  /** After accepted/conflict report: hop to next live once board refreshes. */
  const pendingAdvanceFromRef = useRef<string | null>(null);
  const [resolveNote, setResolveNote] = useState("");
  const [resolveBusy, setResolveBusy] = useState(false);
  const [resolveNotice, setResolveNotice] = useState<string | null>(null);
  const [resolveError, setResolveError] = useState<string | null>(null);
  const [resolveFeedbackMatchId, setResolveFeedbackMatchId] = useState<string | null>(null);
  const [shareCopied, setShareCopied] = useState(false);
  const [embedCopied, setEmbedCopied] = useState(false);
  const [updatedNow, setUpdatedNow] = useState(() => Date.now());
  const cupTournamentId = tournamentIdProp ?? board?.tournamentId ?? null;
  const [cupSmokeOpen, setCupSmokeOpen] = useState(true);
  const [cupSmokeChecks, setCupSmokeChecks] = useState<CupSmokeChecks>(() => ({
    ...CUP_SMOKE_DEFAULTS,
  }));
  const [cupSmokeReady, setCupSmokeReady] = useState(false);
  /** Brief CSS ease for programmatic Fit / select-pan (not pointer pan). */
  const [canvasEase, setCanvasEase] = useState(false);

  const effectivePollMs = useMemo(() => adaptivePollMs(board, pollMs), [board, pollMs]);

  const fetchAudit = useCallback(
    async (opts?: { soft?: boolean }) => {
      const tid = board?.tournamentId;
      if (!tid || isShare) return;
      const soft = Boolean(opts?.soft);
      if (!soft) {
        setAuditLoading(true);
        setAuditError(null);
      }
      try {
        const res = await fetch(`/api/tournaments/${tid}/report/audit`, { cache: "no-store" });
        if (!res.ok) {
          if (!soft) setAuditError(`audit_failed_${res.status}`);
          return;
        }
        const data = (await res.json().catch(() => ({}))) as ReportAuditResponse;
        setAuditEvents(data.events ?? []);
        setConflictLobbyIds(data.conflictLobbyIds ?? []);
        setEvidenceCountsByLobbyId(data.evidenceCountsByLobbyId ?? {});
      } catch {
        if (!soft) setAuditError("network_error");
      } finally {
        if (!soft) setAuditLoading(false);
      }
    },
    [board?.tournamentId, isShare],
  );

  const fetchEvidence = useCallback(async () => {
    const tid = board?.tournamentId;
    if (!tid || isShare) return;
    try {
      const res = await fetch(`/api/tournaments/${tid}/report/evidence`, { cache: "no-store" });
      if (!res.ok) return; // soft-fail (desk-auth / unbound)
      const data = (await res.json().catch(() => ({}))) as { evidence?: EvidenceItem[] };
      setEvidenceItems(Array.isArray(data.evidence) ? data.evidence : []);
    } catch {
      // soft-fail — strip just omits thumbs
    }
  }, [board?.tournamentId, isShare]);

  const runRefresh = useCallback(async () => {
    if (!onRefresh) return;
    setRefreshing(true);
    try {
      await onRefresh();
      // Soft-load conflictLobbyIds + evidence preview on every organize refresh.
      if (isOrganize) {
        await Promise.all([fetchAudit({ soft: true }), fetchEvidence()]);
      }
    } finally {
      setRefreshing(false);
    }
  }, [onRefresh, isOrganize, fetchAudit, fetchEvidence]);

  useEffect(() => {
    if (!isOrganize || !board?.tournamentId) return;
    // Soft-load audit for conflict accents (+ Log tab when open) and evidence preview.
    const t = window.setTimeout(() => {
      void fetchAudit({ soft: tab !== "log" });
      void fetchEvidence();
    }, 0);
    return () => window.clearTimeout(t);
  }, [tab, fetchAudit, fetchEvidence, board?.tournamentId, isOrganize]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target?.closest("input, textarea, select, [contenteditable=true]")) return;

      // Ctrl/Cmd+Z undo reseat (organize); ignore Shift (redo) and typing contexts.
      if (
        (e.metaKey || e.ctrlKey) &&
        !e.altKey &&
        !e.shiftKey &&
        (e.key === "z" || e.key === "Z") &&
        canUndoReseat &&
        onUndoReseat &&
        !reassigning
      ) {
        e.preventDefault();
        void onUndoReseat();
        return;
      }

      if (e.metaKey || e.ctrlKey || e.altKey) return;

      if (e.key === "Escape") {
        if (lightboxSrc) {
          setLightboxSrc(null);
          return;
        }
        if (keysOpen) {
          setKeysOpen(false);
          return;
        }
        if (!selectedMatchId || !onSelectMatch) return;
        onSelectMatch(null);
        return;
      }

      if (!onSelectMatch) return;
      const cycleDown = e.key === "j" || e.key === "ArrowDown";
      const cycleUp = e.key === "k" || e.key === "ArrowUp";
      if (!cycleDown && !cycleUp) return;
      if (!board || board.matches.length === 0) return;
      e.preventDefault();
      const nextId = cycleMatchId(board.matches, selectedMatchId, cycleDown ? 1 : -1);
      if (nextId) onSelectMatch(nextId);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [
    onSelectMatch,
    selectedMatchId,
    board,
    canUndoReseat,
    onUndoReseat,
    reassigning,
    lightboxSrc,
    keysOpen,
  ]);

  // Clear resolve draft when the selected match changes.
  useEffect(() => {
    const t = window.setTimeout(() => setResolveNote(""), 0);
    return () => window.clearTimeout(t);
  }, [selectedMatchId]);

  // Gently center selected match when far off-screen (click or ?match=). Skip mid-pan.
  useEffect(() => {
    if (!selectedMatchId || tab !== "bracket") return;
    if (panning || dragRef.current) return;
    const t = window.setTimeout(() => {
      if (dragRef.current) return;
      const viewport = viewportRef.current;
      if (!viewport) return;
      const node = viewport.querySelector(
        `[data-match-id="${CSS.escape(selectedMatchId)}"]`,
      ) as HTMLElement | null;
      if (!node) return;
      const vr = viewport.getBoundingClientRect();
      const nr = node.getBoundingClientRect();
      const nodeCx = nr.left + nr.width / 2;
      const nodeCy = nr.top + nr.height / 2;
      const margin = 48;
      const centerInView =
        nodeCx >= vr.left + margin &&
        nodeCx <= vr.right - margin &&
        nodeCy >= vr.top + margin &&
        nodeCy <= vr.bottom - margin;
      if (centerInView) return;
      const viewCx = vr.left + vr.width / 2;
      const viewCy = vr.top + vr.height / 2;
      const dx = viewCx - nodeCx;
      const dy = viewCy - nodeCy;
      setCanvasEase(true);
      setTransform((prev) => ({ ...prev, x: prev.x + dx, y: prev.y + dy }));
      window.setTimeout(() => setCanvasEase(false), 200);
    }, 0);
    return () => window.clearTimeout(t);
  }, [selectedMatchId, tab, board, columns, panning]);

  // After accepted/conflict strip report: if reported match left live, select next live.
  useEffect(() => {
    const fromId = pendingAdvanceFromRef.current;
    if (!fromId || !board || !onSelectMatch) return;
    const current = board.matches.find((m) => m.id === fromId);
    if (!current) {
      pendingAdvanceFromRef.current = null;
      return;
    }
    // Still live after refresh — keep selection; clear pending so soft polls don't hop later.
    if (current.status === "live") {
      pendingAdvanceFromRef.current = null;
      return;
    }
    const nextId = nextLiveMatchId(board.matches, fromId);
    pendingAdvanceFromRef.current = null;
    if (!nextId || nextId === fromId) return;
    const t = window.setTimeout(() => onSelectMatch(nextId), 0);
    return () => window.clearTimeout(t);
  }, [board, onSelectMatch]);

  const handleDropSlot = useCallback(
    (to: ReassignTarget) => {
      const from = dragPayloadRef.current;
      if (!from || !onReassign) return;
      if ("teamId" in to && to.teamId && from.lobbyId === to.lobbyId && from.teamId === to.teamId) {
        return;
      }
      void onReassign(from, to);
      dragPayloadRef.current = null;
    },
    [onReassign],
  );

  const onTeamDragStartCapture = useCallback((from: DragPayload) => {
    dragPayloadRef.current = from;
  }, []);

  useEffect(() => {
    if (!effectivePollMs || effectivePollMs <= 0 || !onRefresh) return;
    const timer = setInterval(() => {
      if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
      void runRefresh();
    }, effectivePollMs);
    return () => clearInterval(timer);
  }, [effectivePollMs, onRefresh, runRefresh]);

  // Tick "Updated Xs ago" cheaply (~1s) while we have a successful refresh stamp.
  useEffect(() => {
    if (lastUpdatedAt == null) return;
    const kick = window.setTimeout(() => setUpdatedNow(Date.now()), 0);
    const timer = window.setInterval(() => setUpdatedNow(Date.now()), 1000);
    return () => {
      window.clearTimeout(kick);
      window.clearInterval(timer);
    };
  }, [lastUpdatedAt]);

  const cupKeyRef = useRef<string | null>(null);

  // Hydrate cup-smoke checklist from localStorage (organize only).
  useEffect(() => {
    if (!isOrganize || !cupTournamentId) return;
    const tid = cupTournamentId;
    const stored = readCupSmoke(tid);
    const t = window.setTimeout(() => {
      cupKeyRef.current = tid;
      setCupSmokeOpen(stored.open);
      setCupSmokeChecks(stored.checks);
      setCupSmokeReady(true);
    }, 0);
    return () => {
      window.clearTimeout(t);
      cupKeyRef.current = null;
    };
  }, [isOrganize, cupTournamentId]);

  // Persist cup-smoke only after hydrate applied (avoids overwriting with defaults).
  useEffect(() => {
    if (!isOrganize || !cupTournamentId || !cupSmokeReady) return;
    if (cupKeyRef.current !== cupTournamentId) return;
    writeCupSmoke(cupTournamentId, cupSmokeOpen, cupSmokeChecks);
  }, [isOrganize, cupTournamentId, cupSmokeOpen, cupSmokeChecks, cupSmokeReady]);

  /** Auto-tick cup-smoke when TO does the real action; bumps React state so UI re-reads. */
  const tickCupSmoke = useCallback(
    (key: keyof CupSmokeChecks) => {
      if (!isOrganize || !cupTournamentId) return;
      setCupSmokeChecks((c) => (c[key] ? c : { ...c, [key]: true }));
      softTickCupSmokeKey(cupTournamentId, key);
    },
    [isOrganize, cupTournamentId],
  );

  // Re-show champion banner if a new champion lands this session.
  useEffect(() => {
    const t = window.setTimeout(() => setChampionBannerDismissed(false), 0);
    return () => window.clearTimeout(t);
  }, [board?.championTeamId]);

  useEffect(() => {
    if (!board) return;
    const nextStatuses = new Map(board.matches.map((m) => [m.id, m.status]));
    const prevStatuses = prevStatusesRef.current;
    const prevSeq = lastSeqRef.current;
    lastSeqRef.current = board.seq ?? null;
    prevStatusesRef.current = nextStatuses;

    if (prevStatuses == null) return;

    const changed: string[] = [];
    for (const [id, status] of nextStatuses) {
      if (prevStatuses.get(id) !== status) changed.push(id);
    }
    const seqChanged = prevSeq != null && board.seq != null && prevSeq !== board.seq;
    if (changed.length === 0 && !seqChanged) return;

    // Defer setState so we don't trip react-hooks/set-state-in-effect lint.
    const on = window.setTimeout(() => {
      if (changed.length > 0) setFlashIds(new Set(changed));
      if (seqChanged) setPulse(true);
      setUpdateBanner(
        changed.length > 0
          ? "Results updated · staff Discord alert fires on new reports"
          : "Bracket updated · staff Discord alert fires on new reports",
      );
    }, 0);
    const offFlash = window.setTimeout(() => setFlashIds(new Set()), 2000);
    const offPulse = window.setTimeout(() => setPulse(false), 1200);
    const offBanner = window.setTimeout(() => setUpdateBanner(null), 6000);
    return () => {
      window.clearTimeout(on);
      window.clearTimeout(offFlash);
      window.clearTimeout(offPulse);
      window.clearTimeout(offBanner);
    };
  }, [board]);

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const target = e.target as HTMLElement;
    if (target.closest(".bb-node, .bb-toolbar, .bb-tabs, button, a")) return;
    dragRef.current = {
      pointerId: e.pointerId,
      startX: e.clientX,
      startY: e.clientY,
      origX: transform.x,
      origY: transform.y,
    };
    setPanning(true);
    e.currentTarget.setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== e.pointerId) return;
    setTransform((prev) => ({
      ...prev,
      x: drag.origX + (e.clientX - drag.startX),
      y: drag.origY + (e.clientY - drag.startY),
    }));
  };

  const endPan = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (dragRef.current?.pointerId === e.pointerId) {
      dragRef.current = null;
      setPanning(false);
    }
  };

  const onWheel = (e: ReactWheelEvent<HTMLDivElement>) => {
    e.preventDefault();
    const rect = viewportRef.current?.getBoundingClientRect();
    if (!rect) return;
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    const delta = e.deltaY > 0 ? 0.92 : 1.08;
    setTransform((prev) => {
      const nextK = Math.min(2.4, Math.max(0.4, prev.k * delta));
      const scale = nextK / prev.k;
      return {
        k: nextK,
        x: mx - (mx - prev.x) * scale,
        y: my - (my - prev.y) * scale,
      };
    });
  };

  const withCanvasEase = (fn: () => void) => {
    setCanvasEase(true);
    fn();
    window.setTimeout(() => setCanvasEase(false), 200);
  };

  const resetView = () => withCanvasEase(() => setTransform({ x: 20, y: 16, k: 1 }));

  /** Scale/translate so all columns roughly fit the viewport. */
  const fitView = () => {
    const viewport = viewportRef.current;
    if (!viewport) {
      resetView();
      return;
    }
    const canvas = viewport.querySelector(".bb-canvas") as HTMLElement | null;
    if (!canvas) {
      resetView();
      return;
    }
    const cw = canvas.offsetWidth;
    const ch = canvas.offsetHeight;
    if (cw <= 0 || ch <= 0) {
      resetView();
      return;
    }
    const pad = 24;
    const vw = viewport.clientWidth;
    const vh = viewport.clientHeight;
    const k = Math.min(2.4, Math.max(0.4, Math.min((vw - pad * 2) / cw, (vh - pad * 2) / ch)));
    const x = (vw - cw * k) / 2;
    const y = Math.max(8, (vh - ch * k) / 2);
    withCanvasEase(() => setTransform({ x, y, k }));
  };

  // Programmatic Fit when parent bumps autoFitKey (e.g. post-seed landing).
  useEffect(() => {
    if (autoFitKey == null || autoFitKey <= 0) return;
    if (!board) return;
    const t = window.setTimeout(() => {
      const viewport = viewportRef.current;
      if (!viewport) return;
      const canvas = viewport.querySelector(".bb-canvas") as HTMLElement | null;
      if (!canvas || canvas.offsetWidth <= 0 || canvas.offsetHeight <= 0) {
        withCanvasEase(() => setTransform({ x: 20, y: 16, k: 1 }));
        return;
      }
      const pad = 24;
      const vw = viewport.clientWidth;
      const vh = viewport.clientHeight;
      const cw = canvas.offsetWidth;
      const ch = canvas.offsetHeight;
      const k = Math.min(2.4, Math.max(0.4, Math.min((vw - pad * 2) / cw, (vh - pad * 2) / ch)));
      const x = (vw - cw * k) / 2;
      const y = Math.max(8, (vh - ch * k) / 2);
      withCanvasEase(() => setTransform({ x, y, k }));
    }, 50);
    return () => window.clearTimeout(t);
  }, [autoFitKey, board]);

  // Embed: Fit on load + viewport resize (OBS Browser Source framing).
  useEffect(() => {
    if (!isEmbed || !board) return;
    const runFit = () => {
      const viewport = viewportRef.current;
      if (!viewport) return;
      const canvas = viewport.querySelector(".bb-canvas") as HTMLElement | null;
      if (!canvas || canvas.offsetWidth <= 0 || canvas.offsetHeight <= 0) {
        withCanvasEase(() => setTransform({ x: 20, y: 16, k: 1 }));
        return;
      }
      const pad = 16;
      const vw = viewport.clientWidth;
      const vh = viewport.clientHeight;
      const cw = canvas.offsetWidth;
      const ch = canvas.offsetHeight;
      const k = Math.min(2.4, Math.max(0.4, Math.min((vw - pad * 2) / cw, (vh - pad * 2) / ch)));
      const x = (vw - cw * k) / 2;
      const y = Math.max(8, (vh - ch * k) / 2);
      withCanvasEase(() => setTransform({ x, y, k }));
    };
    const t = window.setTimeout(runFit, 50);
    let resizeTimer: number | null = null;
    const onResize = () => {
      if (resizeTimer != null) window.clearTimeout(resizeTimer);
      resizeTimer = window.setTimeout(runFit, 100);
    };
    window.addEventListener("resize", onResize);
    return () => {
      window.clearTimeout(t);
      if (resizeTimer != null) window.clearTimeout(resizeTimer);
      window.removeEventListener("resize", onResize);
    };
  }, [isEmbed, board]);

  const zoomBy = (factor: number) =>
    setTransform((prev) => ({
      ...prev,
      k: Math.min(2.4, Math.max(0.4, prev.k * factor)),
    }));

  const toggleFullscreen = () => {
    const el = shellRef.current;
    if (!el) return;
    if (document.fullscreenElement) {
      void document.exitFullscreen();
    } else {
      void el.requestFullscreen?.();
    }
  };

  const selected = useMemo(() => {
    if (!board || !selectedMatchId) return null;
    return board.matches.find((m) => m.id === selectedMatchId) ?? null;
  }, [board, selectedMatchId]);

  const selectedEvidence = useMemo(() => {
    if (!selected || isShare) return [] as EvidenceItem[];
    return evidenceItems.filter((ev) => ev.lobbyId === selected.id);
  }, [selected, evidenceItems, isShare]);

  // Derive strip report order from seating (teamIds); TO overrides survive soft polls
  // until the seated set changes. Reset clears override → back to teamIds (not standings).
  const reportOrder = useMemo(() => {
    if (!selected) return [] as string[];
    const base = selected.teamIds;
    const override = reportOverrides[selected.id];
    if (
      override &&
      override.length === base.length &&
      override.every((id) => base.includes(id)) &&
      base.every((id) => override.includes(id))
    ) {
      return override;
    }
    return base.slice();
  }, [selected, reportOverrides]);

  const hasReportOverride = useMemo(() => {
    if (!selected) return false;
    const override = reportOverrides[selected.id];
    const base = selected.teamIds;
    return Boolean(
      override &&
      override.length === base.length &&
      override.every((id) => base.includes(id)) &&
      base.every((id) => override.includes(id)),
    );
  }, [selected, reportOverrides]);

  const canReportStrip = Boolean(
    isOrganize &&
    editable &&
    selected &&
    selected.status === "live" &&
    selected.teamIds.length >= 2 &&
    board?.tournamentId,
  );

  const selectedInConflict = Boolean(
    selected &&
    (conflictLobbyIds.includes(selected.id) ||
      selected.status === "disputed" ||
      selected.status === "conflict"),
  );

  const canResolveStrip = Boolean(
    isOrganize && editable && selected && selectedInConflict && board?.tournamentId,
  );

  const stripReportNotice =
    reportFeedbackMatchId && selectedMatchId === reportFeedbackMatchId ? reportNotice : null;
  const stripReportError =
    reportFeedbackMatchId && selectedMatchId === reportFeedbackMatchId ? reportError : null;
  const stripResolveNotice =
    resolveFeedbackMatchId && selectedMatchId === resolveFeedbackMatchId ? resolveNotice : null;
  const stripResolveError =
    resolveFeedbackMatchId && selectedMatchId === resolveFeedbackMatchId ? resolveError : null;
  const stripReportHint = selected ? reportStripHint(selected.kind, reportOrder.length) : null;

  const submitStripReport = useCallback(async () => {
    if (!board?.tournamentId || !selected || reportOrder.length < 2) return;
    if (reportBusy) return;
    const matchId = selected.id;
    const tournamentId = board.tournamentId;
    const summary = reportOrder
      .map((id, i) => `${ordinal(i + 1)} ${truncateName(teams.get(id)?.name ?? id)}`)
      .join(", ");
    if (!window.confirm(`Submit standings? ${summary}`)) return;
    setReportBusy(true);
    setReportError(null);
    setReportNotice(null);
    setReportFeedbackMatchId(matchId);
    try {
      const res = await fetch(`/api/tournaments/${tournamentId}/report`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          lobbyId: matchId,
          standings: reportOrder,
          role: "to",
        }),
      });
      const data = (await res.json().catch(() => ({}))) as {
        error?: string;
        reportStatus?: string;
      };
      if (!res.ok) {
        setReportError(humanizeReportError(data.error ?? `report_failed_${res.status}`));
        return;
      }
      setReportNotice(reportStatusNotice(data.reportStatus));
      // Drop local override so next derive returns to seating (teamIds).
      setReportOverrides((prev) => {
        if (!(matchId in prev)) return prev;
        const next = { ...prev };
        delete next[matchId];
        return next;
      });
      // Advance TO flow after accepted/conflict; skip duplicate/corroborated (status unchanged).
      if (data.reportStatus === "accepted" || data.reportStatus === "conflict") {
        pendingAdvanceFromRef.current = matchId;
        // Soft-tick cup-smoke "report" checklist for this tournament.
        setCupSmokeChecks((c) => (c.report ? c : { ...c, report: true }));
        softTickCupSmokeKey(cupTournamentId, "report");
      }
      if (onRefresh) await onRefresh();
      await fetchAudit({ soft: true });
    } catch {
      setReportError(humanizeReportError("network_error"));
    } finally {
      setReportBusy(false);
    }
  }, [board, selected, reportOrder, onRefresh, fetchAudit, teams, reportBusy, cupTournamentId]);

  const submitStripResolve = useCallback(async () => {
    if (!board?.tournamentId || !selected) return;
    const matchId = selected.id;
    const tournamentId = board.tournamentId;
    setResolveBusy(true);
    setResolveError(null);
    setResolveNotice(null);
    setResolveFeedbackMatchId(matchId);
    try {
      const res = await fetch(`/api/tournaments/${tournamentId}/report/resolve`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          lobbyId: matchId,
          note: resolveNote.trim() || undefined,
        }),
      });
      const data = (await res.json().catch(() => ({}))) as {
        error?: string;
        alreadyResolved?: boolean;
      };
      if (!res.ok) {
        const msg =
          data.error === "not_in_conflict"
            ? "Lobby is not in conflict (409) — refresh audit."
            : (data.error ?? `resolve_failed_${res.status}`).replace(/_/g, " ");
        setResolveError(msg);
        return;
      }
      setResolveNotice(
        data.alreadyResolved
          ? "Conflict already marked resolved."
          : "Conflict marked resolved (audit only — bracket unchanged).",
      );
      setResolveNote("");
      await fetchAudit({ soft: true });
      if (onRefresh) await onRefresh();
    } catch {
      setResolveError("Network error — check your connection and try again.");
    } finally {
      setResolveBusy(false);
    }
  }, [board, selected, resolveNote, fetchAudit, onRefresh]);

  const copyShareLink = useCallback(async () => {
    const tid = board?.tournamentId;
    if (!tid || typeof window === "undefined") return;
    const url = `${window.location.origin}/tournaments/${tid}/bracket/share`;
    try {
      await navigator.clipboard.writeText(url);
      setShareCopied(true);
      window.setTimeout(() => setShareCopied(false), 2000);
      tickCupSmoke("share");
    } catch {
      setShareCopied(false);
    }
  }, [board?.tournamentId, tickCupSmoke]);

  const copyEmbedUrl = useCallback(async () => {
    const tid = board?.tournamentId;
    if (!tid || typeof window === "undefined") return;
    const url = `${window.location.origin}/tournaments/${tid}/bracket/share?embed=1`;
    try {
      await navigator.clipboard.writeText(url);
      setEmbedCopied(true);
      window.setTimeout(() => setEmbedCopied(false), 2000);
      // Same checklist item as share link (prefer ticking share).
      tickCupSmoke("share");
    } catch {
      setEmbedCopied(false);
    }
  }, [board?.tournamentId, tickCupSmoke]);

  const standingsTeams = useMemo(() => {
    if (!board) return [];
    return board.teams.slice().sort((a, b) => {
      const sa = a.seed ?? Number.POSITIVE_INFINITY;
      const sb = b.seed ?? Number.POSITIVE_INFINITY;
      if (sa !== sb) return sa - sb;
      return a.name.localeCompare(b.name);
    });
  }, [board]);

  const matchSummary = useMemo(() => {
    if (!board) return null;
    let live = 0;
    let complete = 0;
    for (const m of board.matches) {
      if (m.status === "live") live += 1;
      if (m.status === "final" || m.status === "verified") complete += 1;
    }
    return { live, complete, total: board.matches.length };
  }, [board]);

  const liveStreams = useMemo(
    () => (isOrganize ? liveStreamHandles(streamPresence) : []),
    [isOrganize, streamPresence],
  );

  const toChecklist = useMemo(() => {
    if (!isOrganize) return null;
    const live = matchSummary?.live ?? 0;
    const conflicts = conflictLobbyIds.length;
    const readyLobbies =
      readyByLobbyId && board ? countAllReadyLobbies(board.matches, readyByLobbyId) : null;
    let evidenceTotal = 0;
    for (const n of Object.values(evidenceCountsByLobbyId)) evidenceTotal += n;
    return {
      seeded: Boolean(board),
      live,
      conflicts,
      readyLobbies,
      evidenceTotal,
    };
  }, [isOrganize, board, matchSummary, conflictLobbyIds, readyByLobbyId, evidenceCountsByLobbyId]);

  const shellClass = [
    "bb-shell",
    compact ? "bb-shell--compact" : "",
    pulse ? "bb-shell--pulse" : "",
    isEmbed ? "bb-shell--embed" : "",
    className ?? "",
  ]
    .filter(Boolean)
    .join(" ");

  const championId = board?.championTeamId;
  const championName = championId ? (teams.get(championId)?.name ?? championId) : null;
  const showChampionBanner = Boolean(
    championName &&
    (championId || phase?.kind === "final") &&
    !(isOrganize && championBannerDismissed),
  );
  const feedTeamId = board?.feedTeamId;
  const feedTeamName = feedTeamId ? (teams.get(feedTeamId)?.name ?? feedTeamId) : null;
  // Soft LB feed chip: organize + share (incl. embed); not limited to champion-banner-only.
  const showFeedChip = Boolean(feedTeamName) && (isOrganize || isShare);

  return (
    <section ref={shellRef} className={shellClass} aria-label="Bracket board">
      {isEmbed ? (
        <div className="bb-toolbar bb-toolbar--embed" aria-label="Embed chrome">
          <span className="bb-embed-mark">Brakets</span>
          {phase ? <StatusChip kind={phase.kind} label={phase.label} /> : null}
          {showFeedChip ? (
            <span
              className="bb-feed-chip bb-feed-chip--embed"
              title="Lower-bracket feed into Final"
              aria-label={`LB feed ${feedTeamName}`}
            >
              LB feed: {feedTeamName}
            </span>
          ) : null}
          {lastUpdatedAt != null ? (
            <span className="bb-updated" title="Last successful bracket refresh" aria-live="polite">
              {formatUpdatedAgo(lastUpdatedAt, updatedNow)}
            </span>
          ) : null}
        </div>
      ) : null}
      {isEmbed ? null : (
        <div className="bb-toolbar">
          <div className="bb-toolbar__left">
            <p className="bb-eyebrow">Run surface · bracket</p>
            <h2 className="bb-heading">{compact ? "Bracket" : "Tournament bracket"}</h2>
            {board ? (
              <p className="bb-sub">
                {board.format ?? "bracket"} · seq {board.seq ?? "—"} · {board.matches.length}{" "}
                matches
                {board.championTeamId
                  ? ` · Champion ${teams.get(board.championTeamId)?.name ?? board.championTeamId}`
                  : ""}
                {editable ? " · TO drag-to-reseat" : ""}
              </p>
            ) : (
              <p className="bb-sub">
                {editable
                  ? "Pan · zoom · drag teams to reseat (TO override)"
                  : "Pan · zoom · match nodes · snapshot read-only"}
              </p>
            )}
          </div>
          <div className="bb-toolbar__right">
            {phase ? <StatusChip kind={phase.kind} label={phase.label} /> : null}
            {board ? <Tag>{board.status}</Tag> : null}
            <Button type="button" variant="ghost" size="sm" onClick={() => zoomBy(1.15)}>
              Zoom in
            </Button>
            <Button type="button" variant="ghost" size="sm" onClick={() => zoomBy(1 / 1.15)}>
              Zoom out
            </Button>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={fitView}
              title="Fit all columns in view"
            >
              Fit
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={resetView}
              title="Reset pan/zoom"
            >
              Reset
            </Button>
            <div className="bb-keys">
              <Button
                type="button"
                variant="ghost"
                size="sm"
                aria-expanded={keysOpen}
                aria-haspopup="dialog"
                title="Keyboard shortcuts"
                onClick={() => setKeysOpen((v) => !v)}
              >
                Keys
              </Button>
              {keysOpen ? (
                <div className="bb-keys__popover" role="dialog" aria-label="Keyboard shortcuts">
                  <ul className="bb-keys__list">
                    <li>
                      <kbd>j</kbd> / <kbd>k</kbd> — next/prev live match
                    </li>
                    <li>
                      <kbd>Esc</kbd> — clear selection
                    </li>
                    {isOrganize ? (
                      <li>
                        <kbd>Ctrl</kbd>/<kbd>⌘</kbd>+<kbd>Z</kbd> — undo reseat
                      </li>
                    ) : null}
                    <li>
                      <strong>Fit</strong> / <strong>Reset</strong> — framing
                    </li>
                  </ul>
                  <button
                    type="button"
                    className="bb-keys__close"
                    onClick={() => setKeysOpen(false)}
                  >
                    Close
                  </button>
                </div>
              ) : null}
            </div>
            <Button type="button" variant="ghost" size="sm" onClick={toggleFullscreen}>
              Fullscreen
            </Button>
            <Button type="button" variant="ghost" size="sm" onClick={() => window.print()}>
              Printable
            </Button>
            {isOrganize && board?.tournamentId ? (
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={() => void copyShareLink()}
              >
                {shareCopied ? "Copied" : "Copy share link"}
              </Button>
            ) : null}
            {isOrganize && board?.tournamentId ? (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                title="OBS Browser Source URL (?embed=1)"
                onClick={() => void copyEmbedUrl()}
              >
                {embedCopied ? "Copied embed URL" : "Copy embed URL"}
              </Button>
            ) : null}
            {editable && onUndoReseat ? (
              <Button
                type="button"
                variant="secondary"
                size="sm"
                disabled={!canUndoReseat || reassigning}
                title={
                  canUndoReseat ? "Undo last reseat (swap or empty-slot move)" : "No reseat to undo"
                }
                onClick={() => void onUndoReseat()}
              >
                Undo reseat
              </Button>
            ) : null}
            {onRefresh ? (
              <Button
                type="button"
                variant="secondary"
                size="sm"
                disabled={refreshing}
                onClick={() => void runRefresh()}
              >
                {refreshing ? "Refreshing…" : "Refresh"}
              </Button>
            ) : null}
            {lastUpdatedAt != null ? (
              <span
                className="bb-updated"
                title="Last successful bracket refresh"
                aria-live="polite"
              >
                {formatUpdatedAgo(lastUpdatedAt, updatedNow)}
              </span>
            ) : null}
            {effectivePollMs && effectivePollMs > 0 ? (
              <span className="bb-live-note" title="Soft poll while this tab is visible">
                Live · auto-refresh {Math.round(effectivePollMs / 1000)}s
              </span>
            ) : null}
          </div>
        </div>
      )}

      {showChampionBanner ? (
        <div
          className={`bb-champion-banner${isEmbed ? " bb-champion-banner--embed" : ""}`}
          role="status"
          aria-label={`Champion ${championName}`}
        >
          <span className="bb-champion-banner__label">Champion:</span>{" "}
          <strong className="bb-champion-banner__name">{championName}</strong>
          {isOrganize ? (
            <button
              type="button"
              className="bb-champion-banner__dismiss"
              aria-label="Dismiss champion banner"
              onClick={() => setChampionBannerDismissed(true)}
            >
              ×
            </button>
          ) : null}
        </div>
      ) : null}

      {showFeedChip && !isEmbed ? (
        <div
          className="bb-feed-chip"
          role="status"
          title="Lower-bracket feed into Final"
          aria-label={`LB feed ${feedTeamName}`}
        >
          LB feed: {feedTeamName}
        </div>
      ) : null}

      {refreshFailed ? (
        <div className="bb-poll-fail" role="status">
          Couldn&apos;t refresh — retrying
        </div>
      ) : null}

      {toChecklist ? (
        <div className="bb-checklist" aria-label="TO checklist">
          <span className="bb-checklist__chip" title="Bracket board loaded">
            Seeded? {toChecklist.seeded ? "Yes" : "No"}
          </span>
          <span className="bb-checklist__chip" title="Matches with status live">
            Live: {toChecklist.live}
          </span>
          <span className="bb-checklist__chip" title="Soft-flagged standings conflicts">
            Conflicts: {toChecklist.conflicts}
          </span>
          {toChecklist.readyLobbies != null ? (
            <span className="bb-checklist__chip" title="Lobbies where every seated side is ready">
              Ready: {toChecklist.readyLobbies}
            </span>
          ) : null}
          {toChecklist.evidenceTotal > 0 ? (
            <span className="bb-checklist__chip" title="Evidence attachments across lobbies">
              Evidence: {toChecklist.evidenceTotal}
            </span>
          ) : null}
          <span className="bb-checklist__hint" title="j/k cycle live matches">
            j/k live
          </span>
        </div>
      ) : null}

      {isOrganize && cupTournamentId ? (
        <div className="bb-cup-smoke" aria-label="Cup smoke">
          <button
            type="button"
            className="bb-cup-smoke__toggle"
            aria-expanded={cupSmokeOpen}
            onClick={() => setCupSmokeOpen((v) => !v)}
          >
            <span className="bb-cup-smoke__title">Before you go live</span>
            <span className="bb-cup-smoke__chev" aria-hidden="true">
              {cupSmokeOpen ? "▾" : "▸"}
            </span>
          </button>
          {cupSmokeOpen ? (
            <ul className="bb-cup-smoke__list">
              <li className="bb-cup-smoke__item">
                <label className="bb-cup-smoke__label">
                  <input
                    type="checkbox"
                    checked={cupSmokeChecks.walkUp}
                    onChange={(e) => setCupSmokeChecks((c) => ({ ...c, walkUp: e.target.checked }))}
                  />
                  <span>
                    Walk-up roster ready
                    {walkUpHref ? (
                      <>
                        {" "}
                        <Link href={walkUpHref} className="bb-cup-smoke__link">
                          desk
                        </Link>
                      </>
                    ) : null}
                  </span>
                </label>
              </li>
              <li className="bb-cup-smoke__item">
                <label className="bb-cup-smoke__label">
                  <input type="checkbox" checked={Boolean(board)} readOnly disabled />
                  <span>Bracket seeded{board ? " ✓" : ""}</span>
                </label>
              </li>
              <li className="bb-cup-smoke__item">
                <label className="bb-cup-smoke__label">
                  <input
                    type="checkbox"
                    checked={cupSmokeChecks.share}
                    onChange={(e) => setCupSmokeChecks((c) => ({ ...c, share: e.target.checked }))}
                  />
                  <span>Share link copied/opened</span>
                </label>
              </li>
              <li className="bb-cup-smoke__item">
                <label className="bb-cup-smoke__label">
                  <input
                    type="checkbox"
                    checked={cupSmokeChecks.discord}
                    onChange={(e) =>
                      setCupSmokeChecks((c) => ({ ...c, discord: e.target.checked }))
                    }
                  />
                  <span>
                    Discord staff channel linked
                    {discordHref ? (
                      <>
                        {" "}
                        <Link
                          href={discordHref}
                          className="bb-cup-smoke__link"
                          onClick={() => tickCupSmoke("discord")}
                        >
                          discord
                        </Link>
                      </>
                    ) : null}
                  </span>
                </label>
              </li>
              <li className="bb-cup-smoke__item">
                <label className="bb-cup-smoke__label">
                  <input
                    type="checkbox"
                    checked={cupSmokeChecks.widget}
                    onChange={(e) => setCupSmokeChecks((c) => ({ ...c, widget: e.target.checked }))}
                  />
                  <span>
                    Game Bar / widget open
                    {widgetHref ? (
                      <>
                        {" "}
                        <Link
                          href={widgetHref}
                          className="bb-cup-smoke__link"
                          onClick={() => tickCupSmoke("widget")}
                        >
                          widget
                        </Link>
                      </>
                    ) : null}
                  </span>
                </label>
              </li>
              <li className="bb-cup-smoke__item">
                <label className="bb-cup-smoke__label">
                  <input
                    type="checkbox"
                    checked={cupSmokeChecks.report}
                    onChange={(e) => setCupSmokeChecks((c) => ({ ...c, report: e.target.checked }))}
                  />
                  <span>
                    Report once from strip / run desk
                    {runDeskHref ? (
                      <>
                        {" "}
                        <Link href={runDeskHref} className="bb-cup-smoke__link">
                          run desk
                        </Link>
                      </>
                    ) : null}
                  </span>
                </label>
              </li>
            </ul>
          ) : null}
        </div>
      ) : null}

      {liveStreams.length > 0 ? (
        <div className="bb-streams" aria-label="Live player streams">
          <span className="bb-streams__label">Streams: {liveStreams.length} live</span>
          <ul className="bb-streams__list">
            {liveStreams.map((s) => (
              <li key={s.key} className={`bb-streams__chip bb-streams__chip--${s.provider}`}>
                <span className="bb-streams__provider">
                  {s.provider === "twitch" ? "Twitch" : "YouTube"}
                </span>
                <span className="bb-streams__handle" title={s.channelId}>
                  {s.handle}
                </span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      {reassignError ? (
        <div className="bb-reassign-error" role="alert">
          {reassignError}
        </div>
      ) : null}
      {reassigning ? (
        <p className="bb-reassign-status" role="status">
          Reseating teams…
        </p>
      ) : null}

      {updateBanner ? (
        <div className="bb-update-banner" role="status">
          <span>{updateBanner}</span>
          {discordHref ? (
            <Link
              href={discordHref}
              className="bb-update-banner__link"
              onClick={() => tickCupSmoke("discord")}
            >
              Discord desk
            </Link>
          ) : null}
          <button
            type="button"
            className="bb-update-banner__dismiss"
            aria-label="Dismiss update notice"
            onClick={() => setUpdateBanner(null)}
          >
            ×
          </button>
        </div>
      ) : null}

      {isEmbed ? null : (
        <div className="bb-tabs" role="tablist" aria-label="Bracket views">
          {(isShare
            ? ([
                ["bracket", "Bracket"],
                ["standings", "Standings"],
                ["announcements", "Announcements"],
              ] as const)
            : ([
                ["bracket", "Bracket"],
                ["standings", "Standings"],
                ["announcements", "Announcements"],
                ["log", "Log"],
              ] as const)
          ).map(([id, label]) => (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={tab === id}
              className={`bb-tab${tab === id ? " bb-tab--active" : ""}`}
              onClick={() => setTab(id)}
            >
              {label}
            </button>
          ))}
        </div>
      )}

      {tab === "standings" && board ? (
        <div className="bb-standings">
          <div className="bb-standings__head">
            <p className="bb-standings__title">Seeded field</p>
            {matchSummary ? (
              <p className="bb-standings__summary">
                {matchSummary.live} live · {matchSummary.complete} complete · {matchSummary.total}{" "}
                matches
              </p>
            ) : null}
          </div>
          <ol className="bb-standings__list">
            {standingsTeams.map((team) => {
              const isChamp = board.championTeamId === team.id;
              const isFeed = board.feedTeamId === team.id;
              return (
                <li key={team.id} className="bb-standings__row">
                  <span className="bb-standings__seed">
                    {team.seed != null ? `#${team.seed}` : "—"}
                  </span>
                  <span className="bb-standings__name">{team.name}</span>
                  <span className="bb-standings__badges">
                    {isChamp ? <Tag>Champion</Tag> : null}
                    {isFeed ? <Tag>LB feed</Tag> : null}
                  </span>
                </li>
              );
            })}
          </ol>
          {standingsTeams.length === 0 ? (
            <p className="bb-state__body">No teams on this bracket yet.</p>
          ) : null}
        </div>
      ) : null}

      {tab === "announcements" ? (
        <div className="bb-state bb-state--stub">
          <p className="bb-state__title">Announcements</p>
          <p className="bb-state__body">Pin notes later — no schema yet.</p>
          <Button type="button" variant="secondary" size="sm" onClick={() => setTab("bracket")}>
            Back to bracket
          </Button>
        </div>
      ) : null}

      {tab === "log" && isOrganize ? (
        <div className="bb-log">
          <div className="bb-log__head">
            <p className="bb-log__title">Report audit log</p>
            <div className="bb-log__meta">
              {conflictLobbyIds.length > 0 ? (
                <Tag>
                  {conflictLobbyIds.length} conflict{conflictLobbyIds.length === 1 ? "" : "s"}
                </Tag>
              ) : null}
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={auditLoading || refreshing}
                onClick={() => void fetchAudit()}
              >
                {auditLoading ? "Loading…" : "Refresh"}
              </Button>
            </div>
          </div>
          {auditError ? (
            <p className="bb-log__error" role="status">
              Couldn&apos;t load report events — bracket tab still works. Try Refresh.
            </p>
          ) : null}
          {!auditError && auditEvents.length === 0 && !auditLoading ? (
            <p className="bb-log__empty">No report events yet.</p>
          ) : null}
          {auditEvents.length > 0 ? (
            <ul className="bb-log__list">
              {auditEvents.slice(0, 30).map((ev) => {
                const lobbyLabel =
                  board?.matches.find((m) => m.id === ev.lobbyId)?.label ?? truncateId(ev.lobbyId);
                const isConflict =
                  ev.status === "conflict" || conflictLobbyIds.includes(ev.lobbyId);
                return (
                  <li
                    key={ev.id}
                    className={`bb-log__row${isConflict ? " bb-log__row--conflict" : ""}`}
                  >
                    <span className="bb-log__time">{formatAuditTime(ev.createdAt)}</span>
                    <span className="bb-log__lobby" title={ev.lobbyId}>
                      {lobbyLabel}
                    </span>
                    <StatusChip kind={auditStatusKind(ev.status)} label={ev.status} />
                    <span className="bb-log__reporter" title={ev.reportedBy}>
                      {truncateId(ev.reportedBy)}
                    </span>
                  </li>
                );
              })}
            </ul>
          ) : null}
          {auditLoading && auditEvents.length === 0 ? (
            <p className="bb-log__empty">Loading report events…</p>
          ) : null}
        </div>
      ) : null}

      {tab === "standings" && !board ? (
        <div className="bb-state bb-state--stub">
          <p className="bb-state__title">Standings unavailable</p>
          <p className="bb-state__body">Load a bracket snapshot to see the seeded field.</p>
          <Button type="button" variant="secondary" size="sm" onClick={() => setTab("bracket")}>
            Back to bracket
          </Button>
        </div>
      ) : null}

      {tab === "bracket" && error ? (
        <div className="bb-state bb-state--error" role="alert">
          <p className="bb-state__title">Couldn’t load bracket</p>
          <p className="bb-state__body">{humanizeBoardError(error)}</p>
          {onRefresh ? (
            <Button type="button" variant="primary" size="sm" onClick={() => void runRefresh()}>
              Try again
            </Button>
          ) : null}
        </div>
      ) : null}

      {tab === "bracket" && !error && loading && !board ? (
        <div className="bb-state bb-state--loading" aria-busy="true">
          <div className="bb-skeleton">
            <div className="bb-skeleton__col" />
            <div className="bb-skeleton__col" />
            <div className="bb-skeleton__col" />
          </div>
          <p className="bb-state__body">Loading bracket snapshot…</p>
        </div>
      ) : null}

      {tab === "bracket" && !error && !loading && !board ? (
        <div className="bb-state bb-state--empty">
          <p className="bb-state__title">Bracket not seeded yet</p>
          <p className="bb-state__body">
            Seed an 8-team bracket to run this cup (8-team engine for now). Title-agnostic
            single-elim is next.
          </p>
          {emptyPanel ? <div className="bb-seed-wrap">{emptyPanel}</div> : null}
          <div className="bb-state__actions">
            {emptyHref ? (
              <a className="bb-empty-link" href={emptyHref}>
                {emptyLabel ?? "Get started"}
              </a>
            ) : null}
            {walkUpHref ? (
              <Link href={walkUpHref}>
                <Button type="button" variant="secondary" size="sm">
                  Walk-up desk
                </Button>
              </Link>
            ) : null}
          </div>
        </div>
      ) : null}

      {tab === "bracket" && board && selected ? (
        <div className="bb-detail" role="status">
          <div className="bb-detail__row">
            <div className="bb-detail__main">
              <span className="bb-detail__label">{selected.label}</span>
              <StatusChip
                kind={matchPhaseLabel(selected, selectedInConflict).kind}
                label={matchPhaseLabel(selected, selectedInConflict).label}
              />
              {selected.side ? <Tag>{selected.side}</Tag> : null}
              {selected.kind ? <Tag>{selected.kind.replace(/_/g, " ")}</Tag> : null}
              {selected.slot ? <Tag>{selected.slot}</Tag> : null}
              <Tag>
                {selected.teamIds.length}/{matchCapacity(selected)} seated
              </Tag>
            </div>
            {onSelectMatch ? (
              <button
                type="button"
                className="bb-detail__clear"
                aria-label="Clear match selection"
                onClick={() => onSelectMatch(null)}
              >
                ×
              </button>
            ) : null}
          </div>
          <p className="bb-detail__teams">
            {selected.teamIds.length === 0
              ? "No teams seated yet."
              : selected.teamIds
                  .map((id) => {
                    const t = teams.get(id);
                    const seed = t?.seed != null ? `#${t.seed} ` : "";
                    return `${seed}${t?.name ?? id}`;
                  })
                  .join(" · ")}
          </p>
          {selectedEvidence.length > 0 ? (
            <div className="bb-evidence" aria-label="Report evidence">
              <p className="bb-evidence__label">Evidence</p>
              <ul className="bb-evidence__list">
                {selectedEvidence.slice(0, 4).map((ev) => {
                  const isImage = (ev.contentType || "").toLowerCase().startsWith("image/");
                  return (
                    <li key={ev.id} className="bb-evidence__item">
                      {isImage ? (
                        <button
                          type="button"
                          className="bb-evidence__thumb"
                          title="Open evidence"
                          onClick={() => setLightboxSrc(ev.viewPath)}
                        >
                          {/* Desk-auth proxy path — not a public CDN URL */}
                          {/* eslint-disable-next-line @next/next/no-img-element */}
                          <img src={ev.viewPath} alt="" loading="lazy" />
                        </button>
                      ) : (
                        <a
                          className="bb-evidence__link"
                          href={ev.viewPath}
                          target="_blank"
                          rel="noreferrer"
                        >
                          Open evidence
                        </a>
                      )}
                    </li>
                  );
                })}
                {selectedEvidence.length > 4 ? (
                  <li className="bb-evidence__more">+{selectedEvidence.length - 4} more</li>
                ) : null}
              </ul>
            </div>
          ) : null}
          {selected.standings && selected.standings.length > 0 ? (
            <p className="bb-detail__meta">
              Winner{" "}
              <strong>{teams.get(selected.standings[0]!)?.name ?? selected.standings[0]}</strong>
              {selected.standings.length > 1
                ? ` · standings ${selected.standings
                    .map((id) => teams.get(id)?.name ?? id)
                    .join(" → ")}`
                : ""}
            </p>
          ) : null}
          {board.championTeamId &&
          (selected.kind === "final_round" || selected.roundKey === "final") ? (
            <p className="bb-detail__meta">
              Champion{" "}
              <strong>{teams.get(board.championTeamId)?.name ?? board.championTeamId}</strong>
            </p>
          ) : null}
          {selected.status === "reported" || selectedInConflict ? (
            <p className="bb-detail__hint">
              {selectedInConflict
                ? "Conflict soft-flagged — mark resolved when the desk accepts latest standings"
                : "Reported — check run desk / Discord"}
            </p>
          ) : null}
          {canResolveStrip ? (
            <div className="bb-resolve">
              <p className="bb-resolve__title">Mark conflict resolved</p>
              <p className="bb-resolve__lede">
                Soft-clear only — does not rewrite bracket standings.
              </p>
              <div className="bb-resolve__field">
                <label htmlFor={`bb-resolve-note-${selected.id}`}>Note (optional)</label>
                <input
                  id={`bb-resolve-note-${selected.id}`}
                  value={resolveNote}
                  maxLength={500}
                  disabled={resolveBusy}
                  onChange={(e) => setResolveNote(e.target.value)}
                  placeholder="Desk note…"
                />
              </div>
              <div className="bb-resolve__actions">
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  disabled={resolveBusy}
                  onClick={() => void submitStripResolve()}
                >
                  {resolveBusy ? "Resolving…" : "Mark resolved"}
                </Button>
              </div>
              {stripResolveNotice ? (
                <p className="bb-resolve__notice" role="status">
                  {stripResolveNotice}
                </p>
              ) : null}
              {stripResolveError ? (
                <p className="bb-resolve__error" role="alert">
                  {stripResolveError}
                </p>
              ) : null}
            </div>
          ) : null}
          {canReportStrip ? (
            <div className="bb-report">
              <p className="bb-report__title">Report standings (best → worst)</p>
              {stripReportHint ? <p className="bb-report__hint">{stripReportHint}</p> : null}
              <ol className="bb-report__list">
                {reportOrder.map((id, index) => {
                  const place = index + 1;
                  const isFirst = place === 1;
                  return (
                    <li key={id} className="bb-report__row">
                      <span className="bb-report__place">
                        <span
                          className={`bb-report__chip${isFirst ? " bb-report__chip--winner" : ""}`}
                          aria-label={ordinal(place)}
                        >
                          {ordinal(place)}
                        </span>
                        <span className="bb-report__name">{teams.get(id)?.name ?? id}</span>
                      </span>
                      <span className="bb-report__moves">
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          disabled={index === 0 || reportBusy}
                          onClick={() => {
                            if (!selected) return;
                            setReportOverrides((prev) => ({
                              ...prev,
                              [selected.id]: moveItem(reportOrder, index, index - 1),
                            }));
                          }}
                        >
                          Up
                        </Button>
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          disabled={index === reportOrder.length - 1 || reportBusy}
                          onClick={() => {
                            if (!selected) return;
                            setReportOverrides((prev) => ({
                              ...prev,
                              [selected.id]: moveItem(reportOrder, index, index + 1),
                            }));
                          }}
                        >
                          Down
                        </Button>
                      </span>
                    </li>
                  );
                })}
              </ol>
              <div className="bb-report__actions">
                <Button
                  type="button"
                  variant="primary"
                  size="sm"
                  disabled={reportBusy || reportOrder.length < 2}
                  onClick={() => void submitStripReport()}
                >
                  {reportBusy ? "Submitting…" : "Submit report"}
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  disabled={reportBusy || !hasReportOverride}
                  title="Clear order override — return to seating (teamIds)"
                  onClick={() => {
                    if (!selected) return;
                    setReportOverrides((prev) => {
                      if (!(selected.id in prev)) return prev;
                      const next = { ...prev };
                      delete next[selected.id];
                      return next;
                    });
                  }}
                >
                  Reset order
                </Button>
              </div>
              {stripReportNotice ? (
                <p className="bb-report__notice" role="status">
                  {stripReportNotice}
                </p>
              ) : null}
              {stripReportError ? (
                <p className="bb-report__error" role="alert">
                  {stripReportError}
                </p>
              ) : null}
            </div>
          ) : null}
          {runDeskHref || walkUpHref || discordHref ? (
            <div
              className="row"
              style={{ justifyContent: "flex-start", flexWrap: "wrap", gap: "0.4rem" }}
            >
              {runDeskHref ? (
                <Link href={runDeskHref}>
                  <Button type="button" variant="primary" size="sm">
                    Open run desk
                  </Button>
                </Link>
              ) : null}
              {walkUpHref ? (
                <Link href={walkUpHref}>
                  <Button type="button" variant="secondary" size="sm">
                    Walk-up
                  </Button>
                </Link>
              ) : null}
              {discordHref ? (
                <Link href={discordHref} onClick={() => tickCupSmoke("discord")}>
                  <Button type="button" variant="ghost" size="sm">
                    Discord
                  </Button>
                </Link>
              ) : null}
            </div>
          ) : null}
        </div>
      ) : null}

      {tab === "bracket" && board ? (
        <div
          ref={viewportRef}
          className={`bb-viewport${panning ? " bb-viewport--panning" : ""}`}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={endPan}
          onPointerCancel={endPan}
          onWheel={onWheel}
        >
          <div
            className={`bb-canvas${canvasEase ? " bb-canvas--ease" : ""}`}
            style={{
              transform: `translate(${transform.x}px, ${transform.y}px) scale(${transform.k})`,
            }}
          >
            <div className="bb-hint" aria-hidden="true">
              {editable
                ? "Drag background to pan · scroll to zoom · drag ⋮⋮ to swap or drop into empty slots"
                : "Drag to pan · scroll to zoom · click a match"}
            </div>
            <div className="bb-columns-wrap">
              <ConnectorLayer columns={columns} />
              <div className="bb-columns">
                {columns.map((col) => (
                  <div key={col.key} className="bb-column">
                    <div className="bb-column__label">{col.label}</div>
                    <div className="bb-column__stack">
                      {col.matches.map((match) => (
                        <MatchNode
                          key={match.id}
                          match={match}
                          teams={teams}
                          selected={selectedMatchId === match.id}
                          onSelect={(id) => onSelectMatch?.(id)}
                          editable={editable && isOrganize}
                          dragOverKey={dragOverKey}
                          onDragStartSlot={onTeamDragStartCapture}
                          onDragOverSlot={setDragOverKey}
                          onDragLeaveSlot={() => setDragOverKey(null)}
                          onDropSlot={handleDropSlot}
                          reassigning={reassigning}
                          flash={flashIds.has(match.id)}
                          readyRows={readyByLobbyId?.[match.id]}
                          inConflict={
                            conflictLobbyIds.includes(match.id) ||
                            match.status === "disputed" ||
                            match.status === "conflict"
                          }
                          evidenceCount={evidenceCountsByLobbyId[match.id] ?? 0}
                        />
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            </div>
          </div>
        </div>
      ) : null}

      {lightboxSrc ? (
        <div
          className="bb-lightbox"
          role="dialog"
          aria-modal="true"
          aria-label="Evidence preview"
          onClick={() => setLightboxSrc(null)}
        >
          <div className="bb-lightbox__panel" onClick={(e) => e.stopPropagation()}>
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img className="bb-lightbox__img" src={lightboxSrc} alt="Evidence" />
            <div className="bb-lightbox__actions">
              <a className="bb-lightbox__open" href={lightboxSrc} target="_blank" rel="noreferrer">
                Open in tab
              </a>
              <button
                type="button"
                className="bb-lightbox__close"
                onClick={() => setLightboxSrc(null)}
              >
                Close
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </section>
  );
}
