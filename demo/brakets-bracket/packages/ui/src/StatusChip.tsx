import { cx } from "./cx";

export type StatusKind = "live" | "checkin" | "upcoming" | "final" | "ready";

const DEFAULT_LABEL: Record<StatusKind, string> = {
  live: "Live",
  checkin: "Check-in",
  upcoming: "Upcoming",
  final: "Final",
  ready: "Ready",
};

const PULSING: Partial<Record<StatusKind, boolean>> = {
  live: true,
  checkin: true,
};

export interface StatusChipProps {
  kind?: StatusKind;
  label?: string;
  className?: string;
}

/** Match / tournament status pill. The live + check-in dots pulse via CSS. */
export function StatusChip({ kind = "upcoming", label, className }: StatusChipProps) {
  const showDot = kind !== "final";
  return (
    <span className={cx("bk-chip", `bk-chip--${kind}`, className)}>
      {showDot && (
        <span className={cx("bk-chip__dot", PULSING[kind] && "bk-chip__dot--pulse")} aria-hidden="true" />
      )}
      {label ?? DEFAULT_LABEL[kind]}
    </span>
  );
}
