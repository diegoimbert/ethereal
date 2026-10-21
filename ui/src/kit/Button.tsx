import { ChevronDown } from "lucide-react";
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
  /** Opens a dropdown (a popover, a menu): a chevron down after the label. */
  chevron?: boolean;
}

export function Button({ tone, variant, size = "md", active, chevron, className, type = "button", children, ...rest }: ButtonProps) {
  const t = resolveTone(tone, variant);
  return (
    <button type={type} aria-pressed={active} className={clsx("eth-button", `eth-button--${t}`, `eth-button--${size}`, className)} {...rest}>
      {children}
      {chevron && <ChevronDown className="eth-button__chevron" aria-hidden />}
    </button>
  );
}
