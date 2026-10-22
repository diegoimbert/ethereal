import clsx from "clsx";
import { useRef, useState, type InputHTMLAttributes, type KeyboardEvent, type PointerEvent, type Ref } from "react";
import { setDragCursor } from "./dragCursor";
import { useDisabled } from "./readOnly";
import { useReadOnly } from "@/state/viewer";
import type { Size } from "./variants";
import { formatFixed, parseNumber, useT } from "@/i18n";

// ---- TextInput ------------------------------------------------------------------------

export interface TextInputProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "size"> {
  size?: Size;
  /** Marks the field invalid (danger border, `aria-invalid`). */
  invalid?: boolean;
  ref?: Ref<HTMLInputElement>;
}

/**
 * For search and filter fields: no autocorrect, autocapitalization, spellcheck or
 * autocomplete (macOS WebKit otherwise pops its autocorrect suggestion under the text, and
 * a search query is no prose). Spread onto the `<input>`; `TextInput type="search"` has it.
 */
export const NO_AUTOCORRECT = { autoCorrect: "off", autoCapitalize: "off", spellCheck: false, autoComplete: "off" } as const;

export function TextInput({ size = "md", invalid, className, type = "text", readOnly, ...rest }: TextInputProps) {
  // A shared project's viewer: names and values show, nothing is typed into them.
  const viewOnly = useReadOnly();
  return (
    <input
      {...(type === "search" ? NO_AUTOCORRECT : {})}
      type={type}
      readOnly={viewOnly || readOnly}
      aria-invalid={invalid || undefined}
      className={clsx("eth-input", `eth-input--${size}`, className)}
      {...rest}
    />
  );
}

// ---- NumberField ----------------------------------------------------------------------

export interface NumberFieldProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "size" | "value" | "onChange" | "min" | "max" | "step"> {
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
  /**
   * Pixels of vertical drag per `step` (hold and drag up/down to change the value; Shift
   * for tenth steps). Default 4. A press without a drag still focuses the field to type.
   */
  dragPixelsPerStep?: number;
  /** A drag starts (open an undo gesture here: all its changes are then one step). */
  onChangeStart?: () => void;
  /** A drag ends (close the gesture). */
  onChangeEnd?: () => void;
  /**
   * Several values at once that differ: shows "—" (typing still sets one value for all).
   */
  mixed?: boolean;
  /**
   * Relative edits (multi-selection): when set, a drag calls it with the offset from the
   * drag start, and an arrow key with its step, instead of `onChange` (typed values still go
   * to `onChange`). Not clamped: the owner clamps each value it moves.
   */
  onNudge?: (delta: number) => void;
}

const DRAG_THRESHOLD_PX = 3;
/** What a mixed `NumberField` shows. */
const MIXED = "—";

function clamp(v: number, min?: number, max?: number): number {
  if (min !== undefined && v < min) return min;
  if (max !== undefined && v > max) return max;
  return v;
}

/**
 * Numeric text field: type + Enter/blur to commit, ↑/↓ to step, Escape to revert, and hold
 * and drag up/down to change it (like the tempo field).
 */
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
  disabled: disabledProp,
  dragPixelsPerStep = 4,
  onChangeStart,
  onChangeEnd,
  mixed = false,
  onNudge,
  ...rest
}: NumberFieldProps) {
  const t = useT();
  const disabled = useDisabled(disabledProp);
  const [draft, setDraft] = useState<string | null>(null);
  const shown = draft ?? (mixed ? MIXED : formatFixed(value, precision));
  const press = useRef<{ id: number; y: number; start: number; last: number; dragging: boolean } | null>(null);

  const onPointerDown = (e: PointerEvent<HTMLInputElement>) => {
    const el = e.currentTarget;
    if (e.button !== 0 || disabled || document.activeElement === el) return;
    // Not focused yet: this may be a drag. A plain click focuses on release.
    e.preventDefault();
    el.setPointerCapture?.(e.pointerId);
    press.current = { id: e.pointerId, y: e.clientY, start: value, last: onNudge ? 0 : value, dragging: false };
  };
  const onPointerMove = (e: PointerEvent<HTMLInputElement>) => {
    const p = press.current;
    if (!p || p.id !== e.pointerId) return;
    const up = p.y - e.clientY;
    if (!p.dragging) {
      if (Math.abs(up) < DRAG_THRESHOLD_PX) return;
      p.dragging = true;
      setDragCursor("ns-resize");
      setDraft(null);
      onChangeStart?.();
    }
    const unit = e.shiftKey ? step / 10 : step;
    const steps = Math.round(up / dragPixelsPerStep);
    if (onNudge) {
      const delta = Number((steps * unit).toFixed(Math.max(precision, 6)));
      if (delta !== p.last) {
        p.last = delta;
        onNudge(delta);
      }
      return;
    }
    const next = clamp(Number((p.start + steps * unit).toFixed(Math.max(precision, 6))), min, max);
    if (next !== p.last) {
      p.last = next;
      onChange(next);
    }
  };
  const onPointerUp = (e: PointerEvent<HTMLInputElement>) => {
    const p = press.current;
    if (!p || p.id !== e.pointerId) return;
    press.current = null;
    if (p.dragging) {
      setDragCursor(null);
      onChangeEnd?.();
    } else if (e.type === "pointerup") {
      e.currentTarget.focus();
      e.currentTarget.select();
    }
  };

  const commit = () => {
    if (draft === null) return;
    const n = parseNumber(draft);
    setDraft(null);
    if (n !== null) onChange(clamp(n, min, max));
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
      if (onNudge) onNudge(s);
      else onChange(clamp(Number((value + s).toFixed(Math.max(precision, 6))), min, max));
    }
  };

  return (
    <span className={clsx("eth-number", `eth-input--${size}`, disabled && "eth-number--disabled", className)}>
      <input
        className="eth-number__input"
        inputMode="decimal"
        role="spinbutton"
        aria-valuenow={mixed ? undefined : value}
        aria-valuetext={mixed ? t("kit.number.mixed") : undefined}
        aria-valuemin={min}
        aria-valuemax={max}
        disabled={disabled}
        value={shown}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={onKeyDown}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        {...rest}
      />
      {unit && <span className="eth-number__unit">{unit}</span>}
    </span>
  );
}
