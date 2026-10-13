import type { ButtonHTMLAttributes, ReactNode } from "react";
import { cx } from "./cx";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ButtonSize = "sm" | "md" | "lg";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  children?: ReactNode;
}

/**
 * Brakets button — productionized from the CDS reference component.
 * Hover/press/focus live in CSS (`.bk-btn` + `:focus-visible`), not JS state.
 */
export function Button({
  variant = "primary",
  size = "md",
  className,
  type = "button",
  children,
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      className={cx("bk-btn", `bk-btn--${variant}`, `bk-btn--${size}`, className)}
      {...rest}
    >
      {children}
    </button>
  );
}
