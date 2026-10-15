/**
 * Slip editing (pure): Ctrl+Shift+drag (Cmd+Shift on macOS) on a clip's body moves its
 * content under the clip, so the clip keeps its place and length on the timeline and
 * only its content offset changes. A looped clip's loop region moves with the offset, as
 * in Ableton.
 *
 * - The content follows the pointer: dragging right shows earlier content (the offset
 *   decreases). A Repitch clip that is transposed plays faster or slower than the
 *   timeline, so its offset moves by `dx * rate` to keep the audio under the pointer.
 * - Clamp: the content the clip plays (and its loop region) stays inside the source.
 *   For audio that is `[0, media length]`, measured in source seconds with the clip's own
 *   warp, transpose and reverse mapping (`clipSourceMapper`), so it agrees with playback and
 *   the drawn waveform. When the clip covers the whole file there is no room.
 *   For MIDI the content ends at the last note's end.
 * - Snapping: the new offset snaps to the grid step (in beats, at the clip start).
 *
 * One drag commits one `Edit::Batch` (`SetBounds` with the same start and length, plus
 * `SetLoop` for looped clips): one undo step.
 */

import type { Beats, Clip, ClipId, ClipLoop, Command, MediaRef, Note, WarpMarker } from "@/generated";
import { BEATS_EPSILON } from "@/state";
import { cmd } from "@/transport";
import type { EngineTransport } from "@/transport";
import { clipSourceMapper, compileWarp, playsRepitch, semitoneRatio } from "@/domain/warpMap";
import { contentSegments, startOf } from "./clipTime";
import { asCountedStep, boundsOf, keyedStep, type ClipBounds } from "./editMath";

/** The modifiers of a slip drag: Ctrl+Shift, or Cmd+Shift (macOS). */
export function isSlipModifier(e: { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean; altKey?: boolean }): boolean {
  return e.shiftKey && (e.ctrlKey || e.metaKey);
}

/** The cursor shown over a clip body while the slip modifiers are held, and while slipping. */
export const SLIP_CURSOR = "col-resize";

/** Content range `[lo, hi]` (content beats) a clip plays, its loop region included. */
export function coveredContent(clip: Pick<Clip, "length" | "offset" | "looping">): [Beats, Beats] {
  let lo = Infinity;
  let hi = -Infinity;
  for (const s of contentSegments(clip, 0)) {
    lo = Math.min(lo, s.c0);
    hi = Math.max(hi, s.c0 + (s.t1 - s.t0));
  }
  const { enabled, start, end } = clip.looping;
  if (enabled && end - start > BEATS_EPSILON) {
    lo = Math.min(lo, start);
    hi = Math.max(hi, end);
  }
  if (!Number.isFinite(lo)) return [clip.offset, clip.offset];
  return [lo, hi];
}

/**
 * What bounds a clip's content: `end(delta)`, the source position reached by the end of the
 * covered content once slipped by `delta`, against `limit`. Audio: source seconds and the
 * media length. MIDI: content beats and the last note end.
 */
export interface SlipSource {
  end(delta: Beats): number;
  limit: number;
}

/**
 * The offset deltas a clip can slip by, `[min, max]` (`min <= 0 <= max`): the covered
 * content stays at or after content 0 and ends at or before the source's end. A clip that
 * already breaks a bound (a leading gap, a source shorter than the clip) can't slip further
 * that way, and never jumps.
 */
export function slipRange(clip: Pick<Clip, "length" | "offset" | "looping">, source: SlipSource | null): [Beats, Beats] {
  const [lo] = coveredContent(clip);
  const min = lo > 0 ? -lo : 0;
  if (!source) return [min, Infinity];
  const fits = (d: Beats) => source.end(d) <= source.limit + 1e-9;
  if (!fits(0)) return [min, 0];
  let a = 0;
  let b = 1;
  while (fits(b) && b < 1e7) {
    a = b;
    b *= 2;
  }
  if (b >= 1e7 && fits(b)) return [min, b];
  for (let i = 0; i < 60; i++) {
    const m = (a + b) / 2;
    if (fits(m)) a = m;
    else b = m;
  }
  return [min, a];
}

/** The `SlipSource` of an audio clip (null without media). `refBpm`: song tempo at the clip start. */
export function audioSlipSource(
  clip: Clip,
  media: Pick<MediaRef, "frames" | "sample_rate"> | undefined,
  markers: ReadonlyArray<Pick<WarpMarker, "beat" | "source">>,
  refBpm: number,
  kind: EngineTransport["kind"],
): SlipSource | null {
  if (clip.content.type !== "Audio" || !media) return null;
  const content = clip.content;
  const [, hi] = coveredContent(clip);
  return {
    // Repitch pivots on the clip offset: map with the slipped offset as the anchor.
    end: (d) => clipSourceMapper(content, markers, refBpm, clip.offset + d, kind)(hi + d),
    limit: media.frames / Math.max(1, media.sample_rate),
  };
}

/** The `SlipSource` of a MIDI clip: its content ends at the last note's end (or its own end). */
export function midiSlipSource(clip: Clip, notes: ReadonlyArray<Pick<Note, "start" | "duration">>): SlipSource {
  const [, hi] = coveredContent(clip);
  let last = 0;
  for (const n of notes) last = Math.max(last, n.start + n.duration);
  return { end: (d) => hi + d, limit: Math.max(last, hi) };
}

/**
 * Content beats per timeline beat of the pointer: how far the offset moves per beat
 * dragged so the content stays under the pointer. 1, except a transposed Repitch clip.
 */
export function slipRate(clip: Clip, markers: ReadonlyArray<Pick<WarpMarker, "beat" | "source">>, kind: EngineTransport["kind"]): number {
  if (clip.content.type !== "Audio") return 1;
  const t = clip.content.transpose;
  if (!t || !Number.isFinite(t)) return 1;
  return playsRepitch(clip.content, compileWarp(clip.content.warp, markers), kind) ? semitoneRatio(t) : 1;
}

/** A clip as a slip drag sees it: its range and rate, fixed when the drag starts. */
export interface SlipClip {
  clip: Clip;
  range: [Beats, Beats];
  rate: number;
}

/** The bounds of a clip slipped by `delta` (same start and length; the loop moves along). */
export function slippedBounds(clip: Clip, delta: Beats): ClipBounds {
  const b: ClipBounds = { ...boundsOf(clip), offset: clip.offset + delta };
  if (clip.looping.enabled) b.looping = { ...clip.looping, start: clip.looping.start + delta, end: clip.looping.end + delta };
  return b;
}

/**
 * The preview of a slip drag of `dx` timeline beats (right = positive). `snapStep`: the grid
 * step in beats at each clip's start, or null when snapping is off or bypassed.
 */
export function slipPreview(clips: ReadonlyArray<SlipClip>, dx: Beats, snapStep: (clip: Clip) => Beats | null): Map<ClipId, ClipBounds> {
  const out = new Map<ClipId, ClipBounds>();
  for (const { clip, range, rate } of clips) {
    let offset = clip.offset - dx * rate;
    const step = snapStep(clip);
    if (step && step > BEATS_EPSILON) offset = Math.round(offset / step) * step;
    const delta = Math.min(range[1], Math.max(range[0], offset - clip.offset));
    out.set(clip.id, slippedBounds(clip, delta));
  }
  return out;
}

/** Commit a slip drag: one undo step for all the slipped clips. */
export function slipCommand(clips: ReadonlyArray<Clip>, preview: ReadonlyMap<ClipId, ClipBounds>): Command | null {
  const commands: Command[] = [];
  for (const c of clips) {
    const p = preview.get(c.id);
    if (!p || Math.abs(p.offset - c.offset) <= BEATS_EPSILON) continue;
    commands.push(cmd("Clip", { type: "SetBounds", id: c.id, start: startOf(c), length: c.length, offset: p.offset }));
    if (p.looping && c.looping.enabled) commands.push(cmd("Clip", { type: "SetLoop", id: c.id, looping: p.looping satisfies ClipLoop }));
  }
  return asCountedStep(keyedStep("undo.clips.slip", "slip clip content"), commands);
}
