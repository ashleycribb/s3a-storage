/**
 * Preview how an 8-slot team order maps onto Opening Cashout A/B.
 * Matches TournamentEngine.seed: first 4 → openA, next 4 → openB.
 * Empty slots show as "—" so the TO always sees four slots per opening.
 */
export function splitFinalsOpeningPreview(names: string[]): {
  openA: string[];
  openB: string[];
} {
  const slots = Array.from({ length: 8 }, (_, i) => {
    const trimmed = (names[i] ?? "").trim();
    return trimmed.length > 0 ? trimmed : "—";
  });
  return {
    openA: slots.slice(0, 4),
    openB: slots.slice(4, 8),
  };
}
