/** Small helpers shared by the arrangement components (kept out of component files for fast refresh). */

import type { Beats, Clip, Color, Track } from "@/generated";
import { beatsPerBar, snapToGrid, type GridSetting, type TempoMap } from "@/timeline";
import { isArrangementClip, startOf } from "./clipTime";

export function colorCss(color: Color): string {
  return `#${(color & 0xffffff).toString(16).padStart(6, "0")}`;
}

/** Grid choices of the toolbar selector. */
export const GRID_OPTIONS: ReadonlyArray<{ id: string; label: string; grid: GridSetting }> = [
  { id: "adaptive-narrow", label: "Adaptive (narrow)", grid: { type: "Adaptive", density: "narrow", triplet: false } },
  { id: "adaptive-medium", label: "Adaptive", grid: { type: "Adaptive", density: "medium", triplet: false } },
  { id: "adaptive-wide", label: "Adaptive (wide)", grid: { type: "Adaptive", density: "wide", triplet: false } },
  { id: "bar", label: "1 Bar", grid: { type: "Fixed", step: { kind: "bars", bars: 1 }, triplet: false } },
  { id: "1/4", label: "1/4", grid: { type: "Fixed", step: { kind: "beats", beats: 1 }, triplet: false } },
  { id: "1/8", label: "1/8", grid: { type: "Fixed", step: { kind: "beats", beats: 0.5 }, triplet: false } },
  { id: "1/16", label: "1/16", grid: { type: "Fixed", step: { kind: "beats", beats: 0.25 }, triplet: false } },
  { id: "off", label: "Off", grid: { type: "Off" } },
];

/** The bar containing `at`: its start and length. */
export function barAround(tempo: TempoMap, at: Beats): { start: Beats; length: Beats } {
  const start = snapToGrid(Math.max(0, at), { kind: "bars", bars: 1 }, tempo, "floor");
  return { start, length: beatsPerBar(tempo.signatureAt(start)) };
}

/** `"start,length;..."` of the clips inside group `group` (a string, so the selector is stable). */
export function groupSummaryKey(
  tracks: Readonly<Record<string, Track>>,
  clips: Readonly<Record<string, Clip>>,
  group: string,
): string {
  const inside = (t: Track | undefined): boolean => {
    let p = t?.parent ?? null;
    let guard = 0;
    while (p !== null && guard++ < 64) {
      if (p === group) return true;
      p = tracks[p]?.parent ?? null;
    }
    return false;
  };
  return Object.values(clips)
    .filter((c) => isArrangementClip(c) && inside(tracks[c.track]))
    .map((c) => [startOf(c), c.length] as const)
    .sort((a, b) => a[0] - b[0])
    .map(([s, l]) => `${s},${l}`)
    .join(";");
}
