"use client";

import { useMemo } from "react";
import { splitFinalsOpeningPreview } from "@/lib/finals-opening-preview";

type FinalsOpeningPreviewProps = {
  names: string[];
  /** Trailing verb in the hint — "create" on cup form, "seed" on board panel. */
  action?: "create" | "seed";
  /**
   * finals: Opening Cashout A/B (THE FINALS).
   * neutral: First 4 / Next 4 → opening groups (any other title).
   */
  variant?: "finals" | "neutral";
};

export function FinalsOpeningPreview({
  names,
  action = "create",
  variant = "finals",
}: FinalsOpeningPreviewProps) {
  const { openA, openB } = useMemo(() => splitFinalsOpeningPreview(names), [names]);
  const titleA = variant === "finals" ? "Opening Cashout A" : "First 4 → opening group";
  const titleB = variant === "finals" ? "Opening Cashout B" : "Next 4 → opening group";

  return (
    <div className="bb-open-preview" aria-live="polite">
      <p className="bb-open-preview__hint form-meta">
        Order maps to openings — drag/reorder before {action}.
      </p>
      <div className="bb-open-preview__cols">
        <OpeningCard title={titleA} names={openA} />
        <OpeningCard title={titleB} names={openB} />
      </div>
    </div>
  );
}

function OpeningCard({ title, names }: { title: string; names: string[] }) {
  return (
    <div className="bb-open-preview__card">
      <h3 className="bb-open-preview__title">{title}</h3>
      <ol className="bb-open-preview__list">
        {names.map((name, i) => (
          <li key={`${title}-${i}`} className={name === "—" ? "bb-open-preview__empty" : undefined}>
            {name}
          </li>
        ))}
      </ol>
    </div>
  );
}
