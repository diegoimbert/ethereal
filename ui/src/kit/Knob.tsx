import clsx from "clsx";
import type { CSSProperties } from "react";
import { useVerticalDrag } from "./useVerticalDrag";
import type { Size } from "./variants";

export interface KnobProps {
  /** Normalized value, 0..1. */
  value: number;
  onChange?: (value: number) => void;
  /** Pointer drag started (open an undo gesture). */
  onChangeStart?: () => void;
  /** Pointer drag ended (close the gesture). */
  onChangeEnd?: () => void;
  label?: string;
  /** Arc drawn from the center (0.5) instead of from the minimum, e.g. pan. */
  bipolar?: boolean;
  /** Double-click resets to this value. Defaults to 0.5 when bipolar, else none. */
  defaultValue?: number;
  /** Diameter: a size token (`--eth-size-knob-<size>`), or pixels for one-offs. Default "md". */
  size?: Size | number;
  /** Text for screen readers / tooltips, e.g. "-6.0 dB". */
  valueText?: string;
  disabled?: boolean;
  className?: string;
}

const START = -135; // degrees, 0 = up
const SWEEP = 270;
// Geometry in a 100×100 viewBox; strokes are in CSS pixels (non-scaling), see kit.css.
const C = 50;
const R = 42;
const POINTER = R - 12;

function polar(r: number, deg: number): [number, number] {
  const rad = ((deg - 90) * Math.PI) / 180;
  return [C + r * Math.cos(rad), C + r * Math.sin(rad)];
}

function arc(r: number, from: number, to: number): string {
  const [a, b] = from <= to ? [from, to] : [to, from];
  const [x1, y1] = polar(r, a);
  const [x2, y2] = polar(r, b);
  const large = b - a > 180 ? 1 : 0;
  return `M ${x1} ${y1} A ${r} ${r} 0 ${large} 1 ${x2} ${y2}`;
}

/** Rotary control. Drag vertically to change (Shift = fine), double-click to reset. */
export function Knob({
  value,
  onChange,
  onChangeStart,
  onChangeEnd,
  label,
  bipolar = false,
  defaultValue,
  size = "md",
  valueText,
  disabled = false,
  className,
}: KnobProps) {
  const handlers = useVerticalDrag({
    value,
    onChange: disabled ? undefined : onChange,
    sensitivity: 1 / 150,
    defaultValue: defaultValue ?? (bipolar ? 0.5 : undefined),
    onChangeStart,
    onChangeEnd,
  });
  const v = Math.min(1, Math.max(0, value));
  const angle = START + v * SWEEP;
  const origin = bipolar ? START + SWEEP / 2 : START;
  const [px, py] = polar(POINTER, angle);
  const style = typeof size === "number" ? ({ "--knob-size": `${size}px` } as CSSProperties) : undefined;

  return (
    <div
      className={clsx("eth-knob", typeof size === "string" && `eth-knob--${size}`, className)}
      style={style}
      role="slider"
      tabIndex={disabled ? -1 : 0}
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={1}
      aria-valuenow={v}
      aria-valuetext={valueText}
      aria-disabled={disabled || undefined}
      title={valueText}
      {...handlers}
    >
      <svg className="eth-knob__svg" viewBox="0 0 100 100" aria-hidden="true">
        <circle className="eth-knob__body" cx={C} cy={C} r={R} />
        <path className="eth-knob__track" d={arc(R, START, START + SWEEP)} />
        {angle !== origin && <path className="eth-knob__value" d={arc(R, origin, angle)} />}
        <line className="eth-knob__pointer" x1={C} y1={C} x2={px} y2={py} />
      </svg>
      {label && <span className="eth-knob__label">{label}</span>}
    </div>
  );
}
