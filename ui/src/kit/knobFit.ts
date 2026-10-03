// The large-knob center value (ux-followups): the number in the middle of the ring, its unit
// in the gap at the bottom of the arc, both shrunk to fit. Pure helpers + the fitting hook
// used by Knob.tsx (kept out of the component file for fast refresh).
import { useLayoutEffect, type RefObject } from "react";
import { knobGeometry } from "../theme/tokens";

const START = Number(knobGeometry.startAngle);
const SWEEP = Number(knobGeometry.sweep);
const R = Number(knobGeometry.radius);

/** Smallest scale the center value shrinks to before it is allowed to clip. */
export const CENTER_MIN_SCALE = 0.5;

/** Digits fill about this much of a line box (cap height over line height). */
const GLYPH_FILL = 0.6;

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
  // Horizontal distance between the arc ends (viewBox units, 0° = up), as a share of 100.
  const endX = (deg: number) => R * Math.sin((deg * Math.PI) / 180);
  const gap = Math.abs(endX(START + SWEEP) - endX(START)) / 100;
  return { value, unit: Math.max(0, gap * size - 2 * stroke) };
}

export interface CenterFitRefs {
  dial: RefObject<HTMLSpanElement | null>;
  value: RefObject<HTMLSpanElement | null>;
  unit: RefObject<HTMLSpanElement | null>;
}

/**
 * Shrinks the center value (and unit) to fit inside the ring: measured after layout and
 * whenever the dial resizes (device cards size knobs by container width). Writes
 * `--knob-center-scale` (applied to the font size) on each text element directly, so it
 * never re-renders the knob.
 */
export function useCenterFit(text: string | undefined, { dial, value, unit }: CenterFitRefs): void {
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
  }, [text, dial, value, unit]);
}
