import clsx from "clsx";
import { useVerticalDrag } from "./useVerticalDrag";

export interface KnobProps {
  /** Normalized value, 0..1. */
  value: number;
  onChange?: (value: number) => void;
  label?: string;
  /** Arc drawn from the center (0.5) instead of from the minimum, e.g. pan. */
  bipolar?: boolean;
  /** Double-click resets to this value. Defaults to 0.5 when bipolar, else none. */
  defaultValue?: number;
  /** Pixel diameter. */
  size?: number;
  /** Text for screen readers / tooltips, e.g. "-6.0 dB". */
  valueText?: string;
  disabled?: boolean;
  className?: string;
}

const START = -135; // degrees, 0 = up
const SWEEP = 270;

function polar(cx: number, cy: number, r: number, deg: number): [number, number] {
  const rad = ((deg - 90) * Math.PI) / 180;
  return [cx + r * Math.cos(rad), cy + r * Math.sin(rad)];
}

function arc(cx: number, cy: number, r: number, from: number, to: number): string {
  const [a, b] = from <= to ? [from, to] : [to, from];
  const [x1, y1] = polar(cx, cy, r, a);
  const [x2, y2] = polar(cx, cy, r, b);
  const large = b - a > 180 ? 1 : 0;
  return `M ${x1} ${y1} A ${r} ${r} 0 ${large} 1 ${x2} ${y2}`;
}

/** Rotary control. Drag vertically to change (Shift = fine), double-click to reset. */
export function Knob({
  value,
  onChange,
  label,
  bipolar = false,
  defaultValue,
  size = 32,
  valueText,
  disabled = false,
  className,
}: KnobProps) {
  const handlers = useVerticalDrag({
    value,
    onChange: disabled ? undefined : onChange,
    sensitivity: 1 / 150,
    defaultValue: defaultValue ?? (bipolar ? 0.5 : undefined),
  });
  const v = Math.min(1, Math.max(0, value));
  const c = size / 2;
  const r = c - 3;
  const angle = START + v * SWEEP;
  const origin = bipolar ? START + SWEEP / 2 : START;
  const [px, py] = polar(c, c, r - 4, angle);

  return (
    <div
      className={clsx("eth-knob", className)}
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
      <svg width={size} height={size} aria-hidden="true">
        <path className="eth-knob__track" d={arc(c, c, r, START, START + SWEEP)} fill="none" strokeWidth={3} />
        {angle !== origin && (
          <path className="eth-knob__value" d={arc(c, c, r, origin, angle)} fill="none" strokeWidth={3} />
        )}
        <line className="eth-knob__pointer" x1={c} y1={c} x2={px} y2={py} strokeWidth={2} strokeLinecap="round" />
      </svg>
      {label && <span>{label}</span>}
    </div>
  );
}
