import type { CSSProperties, ReactNode } from "react";
import { cx } from "./cx";

export interface TagProps {
  /** Optional accent color — tints text + border (e.g. a game's brand color). */
  color?: string;
  className?: string;
  children?: ReactNode;
}

/** Compact square tag — game codes, regions, seeds. */
export function Tag({ color, className, children }: TagProps) {
  const style = color
    ? ({ color, borderColor: `${color}55` } satisfies CSSProperties)
    : undefined;
  return (
    <span className={cx("bk-tag", className)} style={style}>
      {children}
    </span>
  );
}
