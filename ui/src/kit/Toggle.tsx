import clsx from "clsx";
import type { ReactNode } from "react";
import type { Size } from "./variants";

export interface ToggleProps {
  checked: boolean;
  onChange?: (checked: boolean) => void;
  /** Visible label (also the accessible name). */
  label?: ReactNode;
  /** Accessible name when there is no visible label. */
  "aria-label"?: string;
  size?: Size;
  disabled?: boolean;
  className?: string;
}

/** On/off switch (`role="switch"`). */
export function Toggle({ checked, onChange, label, size = "md", disabled, className, ...aria }: ToggleProps) {
  return (
    <label className={clsx("eth-toggle", `eth-toggle--${size}`, disabled && "eth-toggle--disabled", className)}>
      <button
        type="button"
        role="switch"
        className="eth-toggle__track"
        aria-checked={checked}
        aria-label={aria["aria-label"]}
        disabled={disabled}
        onClick={() => onChange?.(!checked)}
      >
        <span className="eth-toggle__thumb" />
      </button>
      {label !== undefined && <span className="eth-toggle__label">{label}</span>}
    </label>
  );
}
