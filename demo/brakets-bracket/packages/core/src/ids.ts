/**
 * Branded id helpers — opaque strings, validated at the edges.
 * One brand per entity so ids can't be swapped across schemas by accident.
 */
import { z } from "zod";

export const UserId = z.string().min(1).brand<"UserId">();
export type UserId = z.infer<typeof UserId>;

export const RosterId = z.string().min(1).brand<"RosterId">();
export type RosterId = z.infer<typeof RosterId>;

export const TournamentId = z.string().min(1).brand<"TournamentId">();
export type TournamentId = z.infer<typeof TournamentId>;

export const BracketId = z.string().min(1).brand<"BracketId">();
export type BracketId = z.infer<typeof BracketId>;

export const LadderId = z.string().min(1).brand<"LadderId">();
export type LadderId = z.infer<typeof LadderId>;

export const MatchId = z.string().min(1).brand<"MatchId">();
export type MatchId = z.infer<typeof MatchId>;

export const RegistrationId = z.string().min(1).brand<"RegistrationId">();
export type RegistrationId = z.infer<typeof RegistrationId>;

export const PlayerPlatformAccountId = z.string().min(1).brand<"PlayerPlatformAccountId">();
export type PlayerPlatformAccountId = z.infer<typeof PlayerPlatformAccountId>;

export const PlayerStreamLinkId = z.string().min(1).brand<"PlayerStreamLinkId">();
export type PlayerStreamLinkId = z.infer<typeof PlayerStreamLinkId>;
