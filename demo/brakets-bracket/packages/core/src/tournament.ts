/**
 * Tournament -> Bracket -> Match + Ladder domain schemas.
 */
import { z } from "zod";
import { BracketId, LadderId, MatchId, RosterId, TournamentId, UserId } from "./ids";

export const TournamentStatus = z.enum([
  "draft",
  "registration",
  "live",
  "completed",
  "cancelled",
]);
export type TournamentStatus = z.infer<typeof TournamentStatus>;

/** Bracket/ladder eliminator shape used across Tournament.format and Bracket.type. */
export const BracketType = z.enum(["single_elim", "double_elim"]);
export type BracketType = z.infer<typeof BracketType>;

export const EntryType = z.enum(["free", "paid"]);
export type EntryType = z.infer<typeof EntryType>;

export const Tournament = z.object({
  id: TournamentId,
  /** The game — e.g. "valorant", "cs2". */
  title: z.string().min(1),
  name: z.string().min(1),
  /** Team-size / mode format — e.g. "1v1", "2v2", "4v4"; game-dependent. */
  format: z.string().min(1),
  phase: z.string().min(1),
  entryType: EntryType,
  entryCost: z.number().int().nonnegative(),
  prizePoolUsd: z.number().nonnegative(),
  startsAt: z.string().datetime(),
  status: TournamentStatus,
  /** Organizer who created the tournament (users.id). */
  createdBy: UserId.optional(),
});
export type Tournament = z.infer<typeof Tournament>;

export const BracketSeed = z.object({
  seed: z.number().int().positive(),
  subjectType: z.enum(["user", "roster"]),
  subjectId: z.union([UserId, RosterId]),
});
export type BracketSeed = z.infer<typeof BracketSeed>;

export const BracketStatus = z.enum(["pending", "live", "completed"]);
export type BracketStatus = z.infer<typeof BracketStatus>;

export const Bracket = z.object({
  id: BracketId,
  tournamentId: TournamentId,
  type: BracketType,
  seeding: z.array(BracketSeed),
  /** Opaque winners/losers tree */
  tree: z.record(z.string(), z.unknown()),
  status: BracketStatus,
});
export type Bracket = z.infer<typeof Bracket>;

export const RankingRules = z.object({
  /** Points awarded per win/loss/draw; game-dependent, kept free-form. */
  pointsPerWin: z.number(),
  pointsPerLoss: z.number(),
  pointsPerDraw: z.number().optional(),
});
export type RankingRules = z.infer<typeof RankingRules>;

export const Ladder = z.object({
  id: LadderId,
  /** The game — e.g. "valorant", "cs2". */
  title: z.string().min(1),
  name: z.string().min(1),
  entryType: EntryType,
  entryCost: z.number().int().nonnegative(),
  windowStart: z.string().datetime(),
  windowEnd: z.string().datetime(),
  rankingRules: RankingRules,
});
export type Ladder = z.infer<typeof Ladder>;

export const MatchStatus = z.enum(["scheduled", "live", "reported", "verified", "disputed", "final"]);
export type MatchStatus = z.infer<typeof MatchStatus>;

export const MatchParticipant = z.object({
  subjectType: z.enum(["user", "roster"]),
  subjectId: z.union([UserId, RosterId]),
});
export type MatchParticipant = z.infer<typeof MatchParticipant>;

export const ScoreReport = z.object({
  reportedBy: UserId,
  score: z.number().int().nonnegative(),
  // Assuming score is a number representing the score reported
});
export type ScoreReport = z.infer<typeof ScoreReport>;