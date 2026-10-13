/**
 * THE FINALS Promotion (open-bracket) — Cashout lobbies of 4 + lower-bracket feed.
 */
import { z } from "zod";
import { BracketId, MatchId, RosterId, TournamentId, UserId } from "./ids";
import { BracketStatus, MatchStatus, MatchVerificationMethod } from "./tournament";

export const FinalsFormat = z.literal("finals_promotion");
export type FinalsFormat = z.infer<typeof FinalsFormat>;

export const FinalsRoundId = z.enum(["ub_open", "ub_cashout", "lb_open", "lb_feed", "final"]);
export type FinalsRoundId = z.infer<typeof FinalsRoundId>;

export const LobbyKind = z.enum(["cashout", "final_round"]);
export type LobbyKind = z.infer<typeof LobbyKind>;

export const BracketSide = z.enum(["upper", "lower"]);
export type BracketSide = z.infer<typeof BracketSide>;

export const FinalsTeam = z.object({
  id: RosterId,
  name: z.string().min(1),
  seed: z.number().int().positive(),
});
export type FinalsTeam = z.infer<typeof FinalsTeam>;

export const LobbyReport = z.object({
  reportedBy: UserId,
  standings: z.array(RosterId).min(2).max(4),
  reportedAt: z.string().datetime(),
});
export type LobbyReport = z.infer<typeof LobbyReport>;

export const LobbyVerification = z.object({
  method: MatchVerificationMethod,
  verifiedAt: z.string().datetime().optional(),
  verifiedBy: UserId.optional(),
});
export type LobbyVerification = z.infer<typeof LobbyVerification>;

export const FinalsLobby = z.object({
  id: MatchId,
  slot: z.string().min(1),
  round: FinalsRoundId,
  side: BracketSide,
  kind: LobbyKind,
  label: z.string().min(1),
  /** 4 team ids for Cashout; 2–3 for Final Round (upper top-2 + lower-bracket feed). */
  teamIds: z.array(RosterId).max(4),
  status: MatchStatus,
  /** Best -> worst. Set once the lobby is verified. */
  standings: z.array(RosterId).min(2).max(4).optional(),
  reports: z.array(LobbyReport).default([]),
  verification: LobbyVerification.optional(),
});
export type FinalsLobby = z.infer<typeof FinalsLobby>;

export const FinalsSnapshot = z.object({
  tournamentId: TournamentId,
  bracketId: BracketId,
  format: FinalsFormat,
  status: BracketStatus,
  teams: z.array(FinalsTeam).length(8),
  lobbies: z.array(FinalsLobby),
  /** Winner of `lb_feed`, injected into the Final Round (upper). */
  feedTeamId: RosterId.optional(),
  championTeamId: RosterId.optional(),
  seq: z.number().int().nonnegative(),
});
export type FinalsSnapshot = z.infer<typeof FinalsSnapshot>;
