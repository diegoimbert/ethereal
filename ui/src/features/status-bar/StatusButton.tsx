import clsx from "clsx";
import type { ButtonHTMLAttributes, HTMLAttributes, ReactNode } from "react";

export interface StatusButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** Leading icon (a lucide icon, sized by the bar). */
  icon?: ReactNode;
  /** `warn` / `danger` colour the icon (something needs attention); default: dim like the bar. */
  tone?: "warn" | "danger";
}

/**
 * An item of the status bar: an icon and a short text, as a button (it opens what it is
 * about, or a popover above it with the details; spread a `Popover`'s trigger props on it).
 */
export function StatusButton({ icon, tone, className, children, type = "button", ...rest }: StatusButtonProps) {
  return (
    <button type={type} className={clsx("eth-status-bar__button", className)} data-tone={tone} {...rest}>
      {icon}
      {children}
    </button>
  );
}

/** A status bar item with nothing to open: an icon and a short text, announced politely. */
export function StatusText({ icon, tone, className, children, ...rest }: HTMLAttributes<HTMLSpanElement> & Pick<StatusButtonProps, "icon" | "tone">) {
  return (
    <span role="status" className={clsx("eth-status-bar__text", className)} data-tone={tone} {...rest}>
      {icon}
      {children}
    </span>
  );
}
