import clsx from "clsx";
import type { ButtonHTMLAttributes } from "react";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "default" | "primary" | "ghost";
  size?: "md" | "sm";
  /** Toggle state; renders `aria-pressed` and the active style. */
  active?: boolean;
}

export function Button({ variant = "default", size = "md", active, className, type = "button", ...rest }: ButtonProps) {
  return (
    <button
      type={type}
      aria-pressed={active}
      className={clsx(
        "eth-button",
        variant !== "default" && `eth-button--${variant}`,
        size === "sm" && "eth-button--sm",
        className,
      )}
      {...rest}
    />
  );
}
