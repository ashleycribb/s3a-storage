type ZodIssueLike = { path?: (string | number)[]; message?: string };

const CREATE_ERROR_COPY: Record<string, string> = {
  forbidden: "You don't have permission to create a tournament.",
  authentication_required: "Please log in to create a tournament.",
  validation_error: "Check the form fields and try again.",
  eight_team_names_required: "Enter all eight team names.",
  roster_need_eight: "Walk-up roster needs at least 8 unique players to auto-fill seed.",
  already_seeded: "This tournament's bracket is already seeded.",
  invalid_seed: "Couldn't seed — check team names (need exactly 8 unique teams).",
  not_seeded: "Bracket has not been seeded yet.",
  network_error: "Network error — check your connection and try again.",
  create_failed: "Couldn't create the tournament. Try again.",
  seed_failed: "Couldn't seed the bracket. Try again.",
  starts_at_invalid: "Enter a valid start date and time.",
  starts_at_past: "Start time must be in the future.",
};

/** Map API snake_case codes to TO-facing English; prefer first zod issue when present. */
export function humanizeCreateError(
  code: string | undefined,
  issues?: ZodIssueLike[] | null,
): string {
  const issue = issues?.[0];
  if (issue) {
    const path = issue.path?.length ? issue.path.join(".") : null;
    const msg = issue.message?.trim() || "invalid";
    return path ? `${path}: ${msg}` : msg;
  }
  if (code && CREATE_ERROR_COPY[code]) return CREATE_ERROR_COPY[code];
  if (code) return code.replace(/_/g, " ");
  return CREATE_ERROR_COPY.create_failed ?? "Couldn't create the tournament. Try again.";
}

/** Client-side startsAt check before POST (ISO or datetime-local). */
export function validateStartsAtClient(
  raw: string,
  { required }: { required: boolean },
): string | null {
  const trimmed = raw.trim();
  if (!trimmed) {
    return required ? "starts_at_invalid" : null;
  }
  const d = new Date(trimmed);
  if (Number.isNaN(d.getTime())) return "starts_at_invalid";
  if (d.getTime() < Date.now() - 60_000) return "starts_at_past";
  return null;
}
