import clsx from "clsx";
import { useRef, type CSSProperties } from "react";
import { knobGeometry } from "../theme/tokens";
import "./Knob.css";
import { centerParts, useCenterFit } from "./knobFit";
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
  /** MIDI learn hook (`midiTarget()` in features/midi-learn): marks this control as mappable. */
  "data-midi-target"?: string;
}

// Geometry comes from the `knobGeometry` tokens (100×100 viewBox, 0° = up); strokes are
// CSS-pixel tokens (`--knob-stroke`, non-scaling), see kit.css.
const START = Number(knobGeometry.startAngle);
const SWEEP = Number(knobGeometry.sweep);
const C = 50;
const R = Number(knobGeometry.radius);
const POINTER = Number(knobGeometry.pointerLength);
const POINTER_INSET = Number(knobGeometry.pointerInset);

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

/** Center value text shows only on knobs big enough to hold it (lg, or ≥ this many px). */
const CENTER_MIN_PX = 40;
/** Radius of the dot marking the value on the ring (viewBox units). */
const DOT_R = 6;

/**
 * Rotary control: a ring (the value arc from the minimum, or from the center when
 * bipolar) with a dot at the value. Large knobs show the value inside the ring (the unit in
 * the arc's bottom gap, both shrunk to fit); on hover,
 * focus or drag the label below turns into the value readout. Drag vertically to change
 * (Shift = fine), double-click to reset.
 */
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
  "data-midi-target": midiTarget,
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
  const [qx, qy] = polar(POINTER_INSET, angle);
  const [dx, dy] = polar(R, angle);
  const style = typeof size === "number" ? ({ "--knob-size": `${size}px` } as CSSProperties) : undefined;
  const center = valueText !== undefined && (typeof size === "number" ? size >= CENTER_MIN_PX : size === "lg");
  const dialRef = useRef<HTMLSpanElement>(null);
  const valueRef = useRef<HTMLSpanElement>(null);
  const unitRef = useRef<HTMLSpanElement>(null);
  useCenterFit(center ? valueText : undefined, { dial: dialRef, value: valueRef, unit: unitRef });
  const parts = center ? centerParts(valueText) : null;

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
      data-midi-target={midiTarget}
      {...handlers}
    >
      <span ref={dialRef} className="eth-knob__dial">
        <svg className="eth-knob__svg" viewBox="0 0 100 100" aria-hidden="true">
          <circle className="eth-knob__body" cx={C} cy={C} r={R} />
          <path className="eth-knob__track" d={arc(R, START, START + SWEEP)} />
          {angle !== origin && <path className="eth-knob__value" d={arc(R, origin, angle)} />}
          <line className="eth-knob__pointer" x1={qx} y1={qy} x2={px} y2={py} />
          <circle className="eth-knob__dot" cx={dx} cy={dy} r={DOT_R} />
        </svg>
        {parts && (
          <span className="eth-knob__center" aria-hidden="true">
            <span ref={valueRef} className="eth-knob__center-value">
              {parts.value}
            </span>
          </span>
        )}
        {parts?.unit && (
          <span ref={unitRef} className="eth-knob__center-unit" aria-hidden="true">
            {parts.unit}
          </span>
        )}
      </span>
      {label && (
        <span className={clsx("eth-knob__label", valueText !== undefined && "eth-knob__label--readout")} aria-hidden="true">
          <span className="eth-knob__name">{label}</span>
          {valueText !== undefined && <span className="eth-knob__readout">{valueText}</span>}
        </span>
      )}
    </div>
  );
}
