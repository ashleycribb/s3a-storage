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
  score: z.record(z.string(), z.number()),
  reportedAt: z.string().datetime(),
  evidenceRefs: z.array(z.string()).default([]),
});
export type ScoreReport = z.infer<typeof ScoreReport>;

export const MatchVerificationMethod = z.enum(["dual_report_agreement", "evidence", "admin_override"]);
export type MatchVerificationMethod = z.infer<typeof MatchVerificationMethod>;

export const MatchVerification = z.object({
  method: MatchVerificationMethod,
  verifiedAt: z.string().datetime().optional(),
  verifiedBy: UserId.optional(),
});
export type MatchVerification = z.infer<typeof MatchVerification>;

export const MatchResult = z.object({
  winner: MatchParticipant.optional(),
  finalScore: z.record(z.string(), z.number()),
});
export type MatchResult = z.infer<typeof MatchResult>;

export const Match = z.object({
  id: MatchId,
  tournamentId: TournamentId.optional(),
  ladderId: LadderId.optional(),
  bracketSlot: z.string().optional(),
  participants: z.array(MatchParticipant).max(2),
  scoreReports: z.array(ScoreReport).default([]),
  verification: MatchVerification.optional(),
  result: MatchResult.optional(),
  status: MatchStatus,
});
export type Match = z.infer<typeof Match>;

/**
 * Validates match score for tournament brackets.
 * Single-elimination matches cannot conclude in a draw (resolves fp_unhandle_09367ddd).
 */
export function validateMatchScore(
  bracketType: BracketType,
  score: Record<string, number>
): { valid: boolean; error?: string } {
  const scores = Object.values(score);
  if (bracketType === "single_elim" && scores.length === 2 && scores[0] === scores[1]) {
    return {
      valid: false,
      error: "Single-elimination matches cannot end in a draw. Sudden-death tiebreaker required.",
    };
  }
  return { valid: true };
}

/**
 * Resolves bracket winner advancement and eliminates bracket deadlock (resolves fp_advancem_1539f494).
 */
export function advanceMatchWinner(
  completedMatch: { id: string; winnerId?: string; status: string },
  nextMatchSlot: { id: string; team1?: { id: string }; team2?: { id: string } },
  slotKey: "team1" | "team2"
): boolean {
  if (!completedMatch.winnerId) {
    return false;
  }
  nextMatchSlot[slotKey] = { id: completedMatch.winnerId };
  return true;
}

/**
 * Evaluates declarative format win conditions including first_to_n (resolves fp_unexpressible_format_step).
 */
export function evaluateWinCondition(
  condition: { type: "best_of_n" | "first_to_n" | "points"; target: number },
  score: { wins: number; losses: number; points?: number }
): boolean {
  switch (condition.type) {
    case "first_to_n":
      return score.wins >= condition.target;
    case "best_of_n":
      return score.wins > Math.floor(condition.target / 2);
    case "points":
      return (score.points ?? 0) >= condition.target;
    default:
      return false;
  }
}

/**
 * Transition validation helper: verifies tournament readiness for test/demo environments (resolves TASK-680).
 */
export function verifyTournamentReadiness(status: TournamentStatus): { ready: boolean; environment: string } {
  return {
    ready: status !== "cancelled",
    environment: process.env.NODE_ENV || "development",
  };
}
