import clsx from "clsx";
import { useLayoutEffect, useRef, type CSSProperties } from "react";
import { knobGeometry } from "../theme/tokens";
import "./Knob.css";
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

/** Smallest scale the center value shrinks to before it is allowed to clip. */
export const CENTER_MIN_SCALE = 0.5;

/**
 * The center value split into its number and unit ("-18.0 dB" -> "-18.0" + "dB"): the number
 * sits in the middle of the ring and the unit in the gap at the bottom of the arc, so a value
 * with a unit fits a small ring. Text without exactly one inner space has no unit.
 */
export function centerParts(text: string): { value: string; unit: string | null } {
  const t = text.trim();
  const i = t.lastIndexOf(" ");
  if (i <= 0 || t.indexOf(" ") !== i) return { value: t, unit: null };
  return { value: t.slice(0, i), unit: t.slice(i + 1) };
}

/**
 * Scale that fits content of `natural` size into `available`, never above 1 and never
 * below `CENTER_MIN_SCALE`. Unmeasured sizes (0, e.g. in jsdom) keep 1.
 */
export function fitScale(available: number, natural: number): number {
  if (!(natural > 0) || !(available > 0)) return 1;
  return Math.max(CENTER_MIN_SCALE, Math.min(1, available / natural));
}

/** Digits fill about this much of a line box (cap height over line height). */
const GLYPH_FILL = 0.6;

/**
 * Room for the center text of a dial `size` px wide whose arc stroke is `stroke` px:
 * the value gets the chord of the ring's inside at its glyphs' half-height (from a line box
 * `valueHeight` px tall), the unit the width of the arc's bottom gap (between the arc ends,
 * minus their caps).
 */
export function centerRoom(size: number, stroke: number, valueHeight: number): { value: number; unit: number } {
  const inner = (size / 2) * (R / 50) - stroke;
  if (!(inner > 0)) return { value: 0, unit: 0 };
  const half = Math.min(inner, (valueHeight * GLYPH_FILL) / 2);
  const value = 2 * Math.sqrt(inner * inner - half * half);
  const [x1] = polar(R, START + SWEEP);
  const [x2] = polar(R, START);
  const unit = (Math.abs(x1 - x2) / 100) * size - 2 * stroke;
  return { value, unit: Math.max(0, unit) };
}

/**
 * Shrinks the center value (and unit) to fit inside the ring: measured after layout and
 * whenever the dial resizes (device cards size knobs by container width). Writes
 * `--knob-center-scale` on each text element directly, so it never re-renders the knob.
 */
function useCenterFit(text: string | undefined) {
  const dial = useRef<HTMLSpanElement>(null);
  const value = useRef<HTMLSpanElement>(null);
  const unit = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    const d = dial.current;
    if (!d || text === undefined) return;
    const fit = () => {
      const v = value.current;
      // Unlaid-out (hidden, jsdom): nothing to fit, and skip the style read.
      if (!v || d.clientWidth === 0) return;
      const stroke = parseFloat(getComputedStyle(d).getPropertyValue("--knob-stroke")) || 0;
      // The scale shrinks the font, so measured sizes are scaled: undo the current one.
      const natural = (el: HTMLElement) => {
        const k = parseFloat(el.style.getPropertyValue("--knob-center-scale")) || 1;
        return { w: el.offsetWidth / k, h: el.offsetHeight / k };
      };
      const nv = natural(v);
      const room = centerRoom(d.clientWidth, stroke, nv.h);
      v.style.setProperty("--knob-center-scale", fitScale(room.value, nv.w).toFixed(3));
      const u = unit.current;
      if (u) u.style.setProperty("--knob-center-scale", fitScale(room.unit, natural(u).w).toFixed(3));
    };
    fit();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(fit);
    ro.observe(d);
    return () => ro.disconnect();
  }, [text]);
  return { dial, value, unit };
}

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
  const fit = useCenterFit(center ? valueText : undefined);
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
      <span ref={fit.dial} className="eth-knob__dial">
        <svg className="eth-knob__svg" viewBox="0 0 100 100" aria-hidden="true">
          <circle className="eth-knob__body" cx={C} cy={C} r={R} />
          <path className="eth-knob__track" d={arc(R, START, START + SWEEP)} />
          {angle !== origin && <path className="eth-knob__value" d={arc(R, origin, angle)} />}
          <line className="eth-knob__pointer" x1={qx} y1={qy} x2={px} y2={py} />
          <circle className="eth-knob__dot" cx={dx} cy={dy} r={DOT_R} />
        </svg>
        {parts && (
          <span className="eth-knob__center" aria-hidden="true">
            <span ref={fit.value} className="eth-knob__center-value">
              {parts.value}
            </span>
          </span>
        )}
        {parts?.unit && (
          <span ref={fit.unit} className="eth-knob__center-unit" aria-hidden="true">
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
