/**
 * Clip body drawing on a `<canvas>`: audio waveforms from engine peaks and the MIDI note
 * preview. The canvas covers only the visible part of a clip (`[from, to)` in timeline
 * beats), so very long or very zoomed-in clips never need a huge canvas.
 */

import type { Beats, Clip, Note, PeakData } from "@/generated";
import { contentSegments } from "./clipTime";
import { peakRange } from "./peaks";

/** A note placed on the timeline (loops unrolled), clipped to the drawn range. */
export interface NoteRect {
  t0: Beats;
  t1: Beats;
  pitch: number;
  velocity: number;
  muted: boolean;
}

/** Notes of a MIDI clip as timeline rectangles within `[from, to)`. */
export function noteRects(clip: Clip, start: Beats, notes: ReadonlyArray<Note>, from: Beats, to: Beats): NoteRect[] {
  const out: NoteRect[] = [];
  for (const seg of contentSegments(clip, start, from, to)) {
    const c1 = seg.c0 + (seg.t1 - seg.t0);
    for (const n of notes) {
      const ns = n.start;
      const ne = n.start + n.duration;
      if (ne <= seg.c0 || ns >= c1) continue;
      out.push({
        t0: seg.t0 + (Math.max(ns, seg.c0) - seg.c0),
        t1: seg.t0 + (Math.min(ne, c1) - seg.c0),
        pitch: n.pitch,
        velocity: n.velocity,
        muted: n.muted,
      });
    }
  }
  return out;
}

/** Pitch range shown by the preview: the clip's notes, padded to at least one octave. */
export function pitchRange(notes: ReadonlyArray<Note>): { lo: number; hi: number } {
  if (notes.length === 0) return { lo: 60, hi: 72 };
  let lo = 127;
  let hi = 0;
  for (const n of notes) {
    lo = Math.min(lo, n.pitch);
    hi = Math.max(hi, n.pitch);
  }
  const pad = Math.max(0, 12 - (hi - lo)) / 2;
  return { lo: Math.max(0, Math.floor(lo - pad)), hi: Math.min(127, Math.ceil(hi + pad)) };
}

export interface DrawArea {
  /** Timeline range drawn, left to right over `width` css px. */
  from: Beats;
  to: Beats;
  width: number;
  height: number;
}

export function drawNotes(
  ctx: CanvasRenderingContext2D,
  area: DrawArea,
  rects: ReadonlyArray<NoteRect>,
  range: { lo: number; hi: number },
  color: string,
): void {
  const ppb = area.width / Math.max(1e-9, area.to - area.from);
  const rows = range.hi - range.lo + 1;
  const h = Math.max(1, Math.min(6, area.height / rows));
  const usable = area.height - h;
  for (const r of rects) {
    const x = (r.t0 - area.from) * ppb;
    const w = Math.max(1, (r.t1 - r.t0) * ppb - 1);
    const y = usable - ((r.pitch - range.lo) / Math.max(1, rows - 1)) * usable;
    ctx.globalAlpha = r.muted ? 0.3 : 0.55 + 0.45 * r.velocity;
    ctx.fillStyle = color;
    ctx.fillRect(x, y, w, h);
  }
  ctx.globalAlpha = 1;
}

export interface WaveformSource {
  /** Timeline start of the clip. */
  start: Beats;
  clip: Pick<Clip, "length" | "offset" | "looping">;
  sampleRate: number;
  /** Content beat → source seconds (see `sourceSecondsMapper`). */
  toSeconds: (contentBeat: Beats) => number;
  /** Media length in frames (nothing is drawn past it). */
  frames: number;
  /** Reversed clip (clip-editing): source times are on the reversed media, frame `f` reads `frames - f`. */
  reversed?: boolean;
  level: number;
  tile: (index: number) => PeakData | null;
}

/** Draw a centered min/max waveform, one column per css px. Returns false if peaks were missing. */
export function drawWaveform(ctx: CanvasRenderingContext2D, area: DrawArea, src: WaveformSource, color: string): boolean {
  const bpp = (area.to - area.from) / Math.max(1, area.width);
  const mid = area.height / 2;
  const segs = contentSegments(src.clip, src.start, area.from, area.to);
  let complete = true;
  ctx.fillStyle = color;
  ctx.beginPath();
  let si = 0;
  for (let x = 0; x < area.width; x++) {
    const t = area.from + x * bpp;
    while (si < segs.length - 1 && t >= segs[si]!.t1) si++;
    const seg = segs[si];
    if (!seg || t < seg.t0 || t >= seg.t1) continue;
    const c = seg.c0 + (t - seg.t0);
    let f0 = Math.floor(src.toSeconds(c) * src.sampleRate);
    let f1 = Math.ceil(src.toSeconds(c + bpp) * src.sampleRate);
    if (src.reversed) [f0, f1] = [src.frames - f1, src.frames - f0];
    if (f0 >= src.frames || f1 <= 0) continue;
    const p = peakRange(Math.max(0, f0), Math.min(src.frames, f1), src.level, src.tile);
    if (!p) {
      complete = false;
      continue;
    }
    const top = mid - p.max * mid;
    const bottom = mid - p.min * mid;
    ctx.rect(x, top, 1, Math.max(1, bottom - top));
  }
  ctx.fill();
  return complete;
}
