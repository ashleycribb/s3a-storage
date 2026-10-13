/**
 * Title-agnostic bracket board model for Phase A canvas.
 * Finals (Cashout lobbies) is the first consumer — adapters map snapshot → board.
 * Core types stay format-agnostic (rounds / matches / teams / status).
 */

export type BoardMatchStatus =
  | "scheduled"
  | "live"
  | "reported"
  | "verified"
  | "final"
  | "disputed"
  | "upcoming"
  | string;

export type BoardTeam = {
  id: string;
  name: string;
  seed?: number;
};

export type BoardMatch = {
  id: string;
  /** Display label for the node header (lobby label, match name, etc.). */
  label: string;
  /** Column / round key used for layout grouping. */
  roundKey: string;
  /** Human round title (e.g. "Opening", "Final"). */
  roundLabel: string;
  /** Optional side badge (upper / lower) — omit when N/A. */
  side?: string;
  /** Optional kind badge (cashout / final_round / bo3…). */
  kind?: string;
  slot?: string;
  status: BoardMatchStatus;
  teamIds: string[];
  /** Best → worst when known. */
  standings?: string[];
};

export type BoardSnapshot = {
  tournamentId: string;
  bracketId?: string;
  format?: string;
  status: string;
  teams: BoardTeam[];
  matches: BoardMatch[];
  championTeamId?: string;
  feedTeamId?: string;
  seq?: number;
};

/** Finals snapshot shape (subset) — keep local to avoid hard coupling in UI. */
export type FinalsLikeTeam = { id: string; name: string; seed: number };
export type FinalsLikeLobby = {
  id: string;
  slot: string;
  round: string;
  side: string;
  kind: string;
  label: string;
  teamIds: string[];
  status: string;
  standings?: string[];
};
export type FinalsLikeSnapshot = {
  tournamentId: string;
  bracketId: string;
  format: string;
  status: string;
  teams: FinalsLikeTeam[];
  lobbies: FinalsLikeLobby[];
  feedTeamId?: string;
  championTeamId?: string;
  seq: number;
};

const FINALS_ROUND_ORDER = ["ub_open", "ub_cashout", "lb_open", "lb_feed", "final"] as const;

const FINALS_ROUND_LABEL: Record<string, string> = {
  ub_open: "Opening Cashout",
  ub_cashout: "Upper Cashout",
  lb_open: "Lower Open",
  lb_feed: "Lower Feed",
  final: "Final Round",
};

/** Column keys for Finals board — groups mid-bracket upper+lower visually. */
const FINALS_COLUMN_FOR_ROUND: Record<string, string> = {
  ub_open: "open",
  ub_cashout: "mid",
  lb_open: "mid",
  lb_feed: "feed",
  final: "final",
};

const FINALS_COLUMN_LABEL: Record<string, string> = {
  open: "Opening",
  mid: "Cashout · Lower",
  feed: "LB Feed",
  final: "Final",
};

export function finalsRoundLabel(round: string): string {
  return FINALS_ROUND_LABEL[round] ?? round.replace(/_/g, " ");
}

export function boardFromFinalsSnapshot(snapshot: FinalsLikeSnapshot): BoardSnapshot {
  const matches: BoardMatch[] = snapshot.lobbies.map((lobby) => {
    const column = FINALS_COLUMN_FOR_ROUND[lobby.round] ?? lobby.round;
    return {
      id: lobby.id,
      label: lobby.label,
      roundKey: column,
      roundLabel: FINALS_COLUMN_LABEL[column] ?? finalsRoundLabel(lobby.round),
      side: lobby.side,
      kind: lobby.kind,
      slot: lobby.slot,
      status: lobby.status,
      teamIds: lobby.teamIds,
      standings: lobby.standings,
    };
  });

  // Stable column order for Finals; unknown rounds append alphabetically.
  const known = new Set<string>(Object.values(FINALS_COLUMN_FOR_ROUND));
  matches.sort((a, b) => {
    const ai = FINALS_ROUND_ORDER.findIndex((r) => FINALS_COLUMN_FOR_ROUND[r] === a.roundKey);
    const bi = FINALS_ROUND_ORDER.findIndex((r) => FINALS_COLUMN_FOR_ROUND[r] === b.roundKey);
    const aOrd = ai >= 0 ? ai : known.has(a.roundKey) ? 50 : 100;
    const bOrd = bi >= 0 ? bi : known.has(b.roundKey) ? 50 : 100;
    if (aOrd !== bOrd) return aOrd - bOrd;
    return (a.slot ?? a.label).localeCompare(b.slot ?? b.label);
  });

  return {
    tournamentId: snapshot.tournamentId,
    bracketId: snapshot.bracketId,
    format: snapshot.format,
    status: snapshot.status,
    teams: snapshot.teams.map((t) => ({ id: t.id, name: t.name, seed: t.seed })),
    matches,
    championTeamId: snapshot.championTeamId,
    feedTeamId: snapshot.feedTeamId,
    seq: snapshot.seq,
  };
}

export type BoardColumn = {
  key: string;
  label: string;
  matches: BoardMatch[];
};

export function groupBoardColumns(board: BoardSnapshot): BoardColumn[] {
  const order: string[] = [];
  const map = new Map<string, BoardMatch[]>();
  for (const match of board.matches) {
    if (!map.has(match.roundKey)) {
      map.set(match.roundKey, []);
      order.push(match.roundKey);
    }
    map.get(match.roundKey)!.push(match);
  }
  return order.map((key) => ({
    key,
    label: map.get(key)?.[0]?.roundLabel ?? key,
    matches: map.get(key) ?? [],
  }));
}

export function statusToChipKind(
  status: BoardMatchStatus,
): "live" | "upcoming" | "ready" | "final" | "checkin" {
  if (status === "live") return "live";
  if (status === "reported" || status === "disputed") return "ready";
  if (status === "verified" || status === "final") return "final";
  if (status === "scheduled") return "upcoming";
  return "upcoming";
}
