import clsx from "clsx";
import type { ButtonHTMLAttributes } from "react";
import { resolveTone, type Size, type Tone } from "./variants";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** Color treatment. */
  tone?: Tone;
  /** @deprecated Use `tone` ("primary" = "accent"). Kept for existing feature code. */
  variant?: "default" | "primary" | "ghost";
  size?: Size;
  /** Toggle state; renders `aria-pressed` and the active style. */
  active?: boolean;
}

export function Button({ tone, variant, size = "md", active, className, type = "button", ...rest }: ButtonProps) {
  const t = resolveTone(tone, variant);
  return (
    <button
      type={type}
      aria-pressed={active}
      className={clsx("eth-button", `eth-button--${t}`, `eth-button--${size}`, className)}
      {...rest}
    />
  );
}
