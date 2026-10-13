import type { HTMLAttributes, ReactNode } from "react";
import { cx } from "./cx";

export interface CardProps extends HTMLAttributes<HTMLDivElement> {
  /** Draw the teal accent edge (live / featured surfaces). */
  accent?: boolean;
  children?: ReactNode;
}

/** Surface card — hairline border, hover lift, optional teal accent edge. */
export function Card({ accent = false, className, children, ...rest }: CardProps) {
  return (
    <div className={cx("bk-card", accent && "bk-card--accent", className)} {...rest}>
      {children}
    </div>
  );
}
