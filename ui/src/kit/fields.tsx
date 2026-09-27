import clsx from "clsx";
import { useState, type InputHTMLAttributes, type KeyboardEvent } from "react";
import type { Size } from "./variants";

// ---- TextInput ------------------------------------------------------------------------

export interface TextInputProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "size"> {
  size?: Size;
  /** Marks the field invalid (danger border, `aria-invalid`). */
  invalid?: boolean;
}

export function TextInput({ size = "md", invalid, className, type = "text", ...rest }: TextInputProps) {
  return (
    <input
      type={type}
      aria-invalid={invalid || undefined}
      className={clsx("eth-input", `eth-input--${size}`, className)}
      {...rest}
    />
  );
}

// ---- NumberField ----------------------------------------------------------------------

export interface NumberFieldProps
  extends Omit<InputHTMLAttributes<HTMLInputElement>, "size" | "value" | "onChange" | "min" | "max" | "step"> {
  value: number;
  /** Called with the parsed, clamped value on Enter/blur/arrow keys. */
  onChange: (value: number) => void;
  min?: number;
  max?: number;
  /** Arrow-key step (Shift = ×10). Default 1. */
  step?: number;
  /** Decimal places shown. Default 0. */
  precision?: number;
  /** Unit shown after the value (e.g. "BPM", "dB"). */
  unit?: string;
  size?: Size;
}

function clamp(v: number, min?: number, max?: number): number {
  if (min !== undefined && v < min) return min;
  if (max !== undefined && v > max) return max;
  return v;
}

/** Numeric text field: type + Enter/blur to commit, ↑/↓ to step, Escape to revert. */
export function NumberField({
  value,
  onChange,
  min,
  max,
  step = 1,
  precision = 0,
  unit,
  size = "md",
  className,
  disabled,
  ...rest
}: NumberFieldProps) {
  const [draft, setDraft] = useState<string | null>(null);
  const shown = draft ?? value.toFixed(precision);

  const commit = () => {
    if (draft === null) return;
    const n = Number.parseFloat(draft);
    setDraft(null);
    if (Number.isFinite(n)) onChange(clamp(n, min, max));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      commit();
    } else if (e.key === "Escape") {
      setDraft(null);
    } else if (e.key === "ArrowUp" || e.key === "ArrowDown") {
      e.preventDefault();
      const s = (e.shiftKey ? 10 : 1) * step * (e.key === "ArrowUp" ? 1 : -1);
      setDraft(null);
      onChange(clamp(Number((value + s).toFixed(Math.max(precision, 6))), min, max));
    }
  };

  return (
    <span className={clsx("eth-number", `eth-input--${size}`, disabled && "eth-number--disabled", className)}>
      <input
        className="eth-number__input"
        inputMode="decimal"
        role="spinbutton"
        aria-valuenow={value}
        aria-valuemin={min}
        aria-valuemax={max}
        disabled={disabled}
        value={shown}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={onKeyDown}
        {...rest}
      />
      {unit && <span className="eth-number__unit">{unit}</span>}
    </span>
  );
}
