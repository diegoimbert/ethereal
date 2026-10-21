/** Small helpers shared by the arrangement components (kept out of component files for fast refresh). */

import type { Beats, Clip, Color, Track } from "@/generated";
import type { GridSetting, TempoMap } from "@/timeline";
import { displayColorCss, ink } from "@/theme";
import type { MessageKey } from "@/i18n";
import { isArrangementClip, startOf } from "./clipTime";

/** Icon/text colors readable on a colored fill (see `inkOn`; tokens `--eth-ink-*`). */
export const DARK_INK = ink.dark;
export const LIGHT_INK = ink.light;

/** WCAG relative luminance of a 0xRRGGBB color (0 = black, 1 = white). */
export function luminance(color: Color): number {
  const lin = (c: number) => {
    const v = c / 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin((color >> 16) & 0xff) + 0.7152 * lin((color >> 8) & 0xff) + 0.0722 * lin(color & 0xff);
}

/** WCAG contrast ratio between two luminances (1–21). */
export function contrastRatio(a: number, b: number): number {
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

/** Dark or light ink on a `color` fill: whichever has the higher WCAG contrast. */
export function inkOn(color: Color): string {
  const l = luminance(color);
  const dark = contrastRatio(l, luminance(parseInt(DARK_INK.slice(1), 16)));
  const light = contrastRatio(l, 1);
  return dark >= light ? DARK_INK : LIGHT_INK;
}

/** A track / clip color as shown (`displayColor`: an imported color in the palette's range). */
export function colorCss(color: Color): string {
  return displayColorCss(color);
}

/** Grid choices of the toolbar selector (`label`: a message key). */
export const GRID_OPTIONS: ReadonlyArray<{ id: string; label: MessageKey; grid: GridSetting }> = [
  { id: "adaptive-narrow", label: "arrangement.grid.adaptive_narrow", grid: { type: "Adaptive", density: "narrow", triplet: false } },
  { id: "adaptive-medium", label: "arrangement.grid.adaptive", grid: { type: "Adaptive", density: "medium", triplet: false } },
  { id: "adaptive-wide", label: "arrangement.grid.adaptive_wide", grid: { type: "Adaptive", density: "wide", triplet: false } },
  { id: "bar", label: "arrangement.grid.bar", grid: { type: "Fixed", step: { kind: "bars", bars: 1 }, triplet: false } },
  { id: "1/4", label: "arrangement.grid.quarter", grid: { type: "Fixed", step: { kind: "beats", beats: 1 }, triplet: false } },
  { id: "1/8", label: "arrangement.grid.eighth", grid: { type: "Fixed", step: { kind: "beats", beats: 0.5 }, triplet: false } },
  { id: "1/16", label: "arrangement.grid.sixteenth", grid: { type: "Fixed", step: { kind: "beats", beats: 0.25 }, triplet: false } },
  { id: "off", label: "common.off", grid: { type: "Off" } },
];

/** The bar containing `at`: its start and length (shorter for a partial bar). */
export function barAround(tempo: TempoMap, at: Beats): { start: Beats; length: Beats } {
  const bar = tempo.barAt(Math.max(0, at));
  return { start: bar.beats, length: tempo.nextBar(bar).beats - bar.beats };
}

/**
 * What a group lane summarizes: the arrangement clips of the tracks inside `group` (at any
 * depth), as `[start, length]` spans in beats, sorted, overlapping or touching spans merged
 * (a group of many tracks draws a few bars, not every clip).
 */
export function groupSummarySpans(tracks: Readonly<Record<string, Track>>, clips: Readonly<Record<string, Clip>>, group: string): Array<[Beats, Beats]> {
  const inside = new Map<string, boolean>();
  const isInside = (id: string): boolean => {
    const known = inside.get(id);
    if (known !== undefined) return known;
    let p = tracks[id]?.parent ?? null;
    let guard = 0;
    let found = false;
    while (p !== null && guard++ < 64) {
      if (p === group) {
        found = true;
        break;
      }
      p = tracks[p]?.parent ?? null;
    }
    inside.set(id, found);
    return found;
  };
  const spans = Object.values(clips)
    .filter((c) => isArrangementClip(c) && isInside(c.track))
    .map((c) => [startOf(c), c.length] as [Beats, Beats])
    .sort((a, b) => a[0] - b[0]);
  const merged: Array<[Beats, Beats]> = [];
  for (const [s, l] of spans) {
    const last = merged[merged.length - 1];
    if (last && s <= last[0] + last[1]) last[1] = Math.max(last[1], s + l - last[0]);
    else merged.push([s, l]);
  }
  return merged;
}

/**
 * The clip color as drawn on canvases (notes, waveform, painted small clips): as is in the
 * dark theme, darkened in the light theme (matches `--clip-strong` in arrangement.css).
 */
export function clipInk(hex: string, theme: "dark" | "light"): string {
  if (theme !== "light") return hex;
  const n = parseInt(hex.slice(1), 16);
  const c = (shift: number) => Math.round(((n >> shift) & 0xff) * 0.7);
  return `#${[c(16), c(8), c(0)].map((v) => v.toString(16).padStart(2, "0")).join("")}`;
}
