import clsx from "clsx";
import type { ButtonHTMLAttributes, ReactNode } from "react";
import { resolveTone, type Size, type Tone } from "./variants";

export interface IconButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> {
  /** Accessible name (required: the button has no text). Also used as the tooltip. */
  label: string;
  icon: ReactNode;
  tone?: Tone;
  size?: Size;
  active?: boolean;
}

/** Square button showing only an icon/glyph. */
export function IconButton({ label, icon, tone, size = "md", active, className, type = "button", title, ...rest }: IconButtonProps) {
  return (
    <button
      type={type}
      aria-label={label}
      aria-pressed={active}
      title={title ?? label}
      className={clsx(
        "eth-button",
        "eth-icon-button",
        `eth-button--${resolveTone(tone, undefined)}`,
        `eth-button--${size}`,
        className,
      )}
      {...rest}
    >
      <span className="eth-icon-button__icon" aria-hidden="true">
        {icon}
      </span>
    </button>
  );
}
