/**
 * Clip edit math and command building (pure): drag previews for move / resize-start /
 * resize-end, and the commands for move, copy, resize, split, duplicate, delete and loop.
 *
 * A drag previews locally and commits once, on release: one command (or one `Edit::Batch`)
 * per drag, so every drag is one undo step.
 */

import type { Beats, Clip, ClipId, ClipLoop, Command, TextKey, TrackId } from "@/generated";
import { BEATS_EPSILON } from "@/state";
import { cmd } from "@/transport";
import { endOf, startOf } from "./clipTime";
import { acceptsClip, type Row } from "./layout";

/** Shortest clip a resize can produce. */
export const MIN_CLIP_LENGTH: Beats = 1 / 16;

export type DragMode = "move" | "resize-start" | "resize-end";

/** Where a clip is (or would be after a drag). */
export interface ClipBounds {
  track: TrackId;
  start: Beats;
  length: Beats;
  offset: Beats;
  /** The loop region, when a drag moves it (slip edits of looped clips, slip.ts). */
  looping?: ClipLoop;
  /**
   * A stretch drag's time-scale factor of the content (new length / old length): the
   * preview draws the notes and the audio `stretch` times as long. `offset` and `looping`
   * are already scaled.
   */
  stretch?: number;
}

export function boundsOf(clip: Clip): ClipBounds {
  return { track: clip.track, start: startOf(clip), length: clip.length, offset: clip.offset };
}

export interface DragInput {
  /** The dragged clips, as they were when the drag started. */
  clips: ReadonlyArray<Clip>;
  /** The clip under the pointer; snapping applies to its edge. */
  anchor: ClipId;
  /** Visible rows (for moves between tracks). */
  rows: ReadonlyArray<Row>;
  /** Snap a timeline position (identity when snapping is off/bypassed). */
  snap: (beats: Beats) => Beats;
  /**
   * Snap a move's delta for an item that started at `origin` (relative snapping: an
   * off-grid clip keeps its offset within the grid, `snapDelta`). Without it, a move snaps
   * the anchor's start through `snap` (absolute).
   */
  snapMove?: (origin: Beats, delta: Beats) => Beats;
  /** Media length in content beats for audio clips (limits unlooped resizes), else null. */
  sourceLength?: (clip: Clip) => Beats | null;
}

/** New bounds of every dragged clip for a pointer delta (beats, rows). */
export function dragPreview(input: DragInput, mode: DragMode, deltaBeats: Beats, rowDelta = 0): Map<ClipId, ClipBounds> {
  const { clips, snap } = input;
  const anchor = clips.find((c) => c.id === input.anchor) ?? clips[0];
  const out = new Map<ClipId, ClipBounds>();
  if (!anchor) return out;

  if (mode === "move") {
    const a = startOf(anchor);
    let d = input.snapMove ? input.snapMove(a, deltaBeats) : snap(a + deltaBeats) - a;
    const minStart = Math.min(...clips.map(startOf));
    if (minStart + d < 0) d = -minStart;
    const tracks = retarget(input, rowDelta);
    for (const c of clips) out.set(c.id, { ...boundsOf(c), start: startOf(c) + d, track: tracks.get(c.id) ?? c.track });
    return out;
  }

  if (mode === "resize-end") {
    const a = endOf(anchor);
    const d = snap(a + deltaBeats) - a;
    for (const c of clips) {
      let length = Math.max(MIN_CLIP_LENGTH, c.length + d);
      const src = c.looping.enabled ? null : (input.sourceLength?.(c) ?? null);
      if (src !== null) length = Math.min(length, Math.max(MIN_CLIP_LENGTH, src - c.offset));
      out.set(c.id, { ...boundsOf(c), length });
    }
    return out;
  }

  // resize-start: the end stays put, the content offset follows the start. Dragging past
  // the content start is allowed: see `shiftedOffset`.
  const a = startOf(anchor);
  const d = snap(a + deltaBeats) - a;
  for (const c of clips) {
    const s0 = startOf(c);
    const end = s0 + c.length;
    const s = Math.min(Math.max(0, s0 + d), end - MIN_CLIP_LENGTH);
    out.set(c.id, { track: c.track, start: s, length: end - s, offset: shiftedOffset(c, s - s0) });
  }
  return out;
}

/** The modifiers that turn an edge drag into a stretch: Ctrl+Shift, or Cmd+Shift (macOS). */
export function isStretchModifier(e: { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean }): boolean {
  return e.shiftKey && (e.ctrlKey || e.metaKey);
}

/** Most a stretch drag speeds up or slows down a clip (the engine's `MAX_STRETCH`). */
const MAX_STRETCH = 1024;

/**
 * New bounds of every dragged clip for a stretch drag of an edge (`isStretchModifier`):
 * like a resize, the other edge stays put, but the content scales with the length (no
 * media limit: the audio plays slower instead) and the bounds carry the factor.
 */
export function stretchPreview(input: DragInput, mode: Exclude<DragMode, "move">, deltaBeats: Beats): Map<ClipId, ClipBounds> {
  const { clips, snap } = input;
  const anchor = clips.find((c) => c.id === input.anchor) ?? clips[0];
  const out = new Map<ClipId, ClipBounds>();
  if (!anchor) return out;
  const atEnd = mode === "resize-end";
  const a = atEnd ? endOf(anchor) : startOf(anchor);
  const d = snap(a + deltaBeats) - a;
  for (const c of clips) {
    const s0 = startOf(c);
    const lo = Math.max(MIN_CLIP_LENGTH, c.length / MAX_STRETCH);
    const hi = c.length * MAX_STRETCH;
    const length = atEnd ? Math.min(hi, Math.max(lo, c.length + d)) : Math.min(hi, Math.max(lo, c.length - Math.max(d, -s0)));
    const start = atEnd ? s0 : s0 + c.length - length;
    const k = length / c.length;
    const { looping } = c;
    out.set(c.id, { track: c.track, start, length, offset: c.offset * k, looping: { ...looping, start: looping.start * k, end: looping.end * k }, stretch: k });
  }
  return out;
}

/** Commit a stretch drag (`ClipCommand::Stretch`, one undo step). */
export function stretchCommand(clips: ReadonlyArray<Clip>, preview: ReadonlyMap<ClipId, ClipBounds>): Command | null {
  const commands: Command[] = [];
  for (const c of clips) {
    const p = preview.get(c.id);
    if (!p || !changed(boundsOf(c), p)) continue;
    commands.push(cmd("Clip", { type: "Stretch", id: c.id, start: p.start, length: p.length }));
  }
  return asCountedStep(countedStep("undo.clips.stretch", commands.length, "stretch clip", "stretch clips"), commands);
}

/**
 * The content offset after moving a clip's start by `delta` beats (end fixed).
 *
 * - Unlooped: plain `offset + delta`. Below 0 it is a leading gap: the clip starts with
 *   empty space before its content (no notes, silence before the audio). Moving the start
 *   back right trims the gap first, then the content.
 * - Looped, starting inside the loop region: the offset wraps within `[loop.start,
 *   loop.end)`, so extending left shows the loop's tail before the loop start and the loop
 *   region itself never moves.
 * - Looped with an intro (offset before the loop start): the intro's own content extends
 *   left down to content 0, then a leading gap, like an unlooped clip; once the offset
 *   reaches the loop it wraps as above.
 */
export function shiftedOffset(c: Pick<Clip, "offset" | "looping">, delta: Beats): Beats {
  const o = c.offset + delta;
  const { enabled, start: ls, end: le } = c.looping;
  const len = le - ls;
  if (!enabled || len <= BEATS_EPSILON) return o;
  if (o < ls - BEATS_EPSILON && c.offset < ls - BEATS_EPSILON) return o;
  if (o >= ls - BEATS_EPSILON && o < le - BEATS_EPSILON) return Math.max(ls, o);
  const w = ls + ((((o - ls) % len) + len) % len);
  return w >= le - BEATS_EPSILON ? ls : w;
}

/** Target track per clip for a vertical move, or the current tracks if any target is invalid. */
function retarget(input: DragInput, rowDelta: number): Map<ClipId, TrackId> {
  const out = new Map<ClipId, TrackId>();
  if (rowDelta === 0) return out;
  const index = new Map(input.rows.map((r, i) => [r.track.id, i]));
  for (const c of input.clips) {
    const i = index.get(c.track);
    const target = i === undefined ? undefined : input.rows[i + rowDelta];
    if (!target || !acceptsClip(target.track, c.content.type)) return new Map();
    out.set(c.id, target.track.id);
  }
  return out;
}

function changed(a: ClipBounds, b: ClipBounds): boolean {
  return (
    a.track !== b.track ||
    Math.abs(a.start - b.start) > BEATS_EPSILON ||
    Math.abs(a.length - b.length) > BEATS_EPSILON ||
    Math.abs(a.offset - b.offset) > BEATS_EPSILON
  );
}

/** One command, or an `Edit::Batch` (one undo step) for several. `null` when empty. */
export function asOneStep(label: string, commands: Command[], key?: TextKey): Command | null {
  if (commands.length === 0) return null;
  if (commands.length === 1) return commands[0]!;
  return cmd("Edit", { type: "Batch", label, commands, ...(key ? { key } : {}) });
}

/** An undo label counted over `n` items: its English text (agents, logs) and its catalog message (`{count, plural, …}`). */
export interface StepLabel {
  text: string;
  key: TextKey;
}

export function countedStep(id: string, n: number, one: string, other: string): StepLabel {
  return { text: n === 1 ? one : other, key: { id, args: { count: n } } };
}

/** An undo label: its English text (agents, logs) and its catalog message (`undo.*`). */
export function keyedStep(id: string, text: string, args: TextKey["args"] = {}): StepLabel {
  return { text, key: { id, args } };
}

/** An `Edit::Batch` (one undo step) of `commands`, even a single one, with a keyed label. */
export function batchStep(label: StepLabel, commands: Command[]): Command {
  return cmd("Edit", { type: "Batch", label: label.text, key: label.key, commands });
}

/** `asOneStep` with a counted label. */
export function asCountedStep(label: StepLabel, commands: Command[]): Command | null {
  return asOneStep(label.text, commands, label.key);
}

/**
 * Where to create a copy that is headed for another track: after every clip on its source
 * track. `Clip::Duplicate` places the copy on the source track and resolves overlaps
 * there at once, so a copy created at its final time could trim or delete the clips it
 * lands on (the original included) before it moves away. Parked clear of them, the copy
 * then moves to its destination, and overlaps resolve on that track only. Call it once per
 * copy: each call reserves room for the returned copy.
 */
export function parkingSpots(clips: Iterable<Clip>): (clip: Clip) => Beats {
  const ends = new Map<TrackId, Beats>();
  for (const c of clips) ends.set(c.track, Math.max(ends.get(c.track) ?? 0, startOf(c) + c.length));
  return (clip) => {
    const at = (ends.get(clip.track) ?? 0) + 1;
    ends.set(clip.track, at + clip.length);
    return at;
  };
}

/**
 * Commit a move (or a copy when `copy`) drag. `all` is every clip of the project (for
 * copies between tracks, see `parkingSpots`); it defaults to the dragged clips.
 */
export function moveCommand(
  clips: ReadonlyArray<Clip>,
  preview: ReadonlyMap<ClipId, ClipBounds>,
  copy: boolean,
  newId: () => string,
  all: Iterable<Clip> = clips,
): Command | null {
  if (!copy) {
    const moves = clips
      .filter((c) => {
        const p = preview.get(c.id);
        return p && changed(boundsOf(c), p);
      })
      .map((c) => {
        const p = preview.get(c.id)!;
        return { id: c.id, track: p.track, start: p.start };
      });
    return moves.length ? cmd("Clip", { type: "Move", moves }) : null;
  }
  const park = parkingSpots(all);
  const commands: Command[] = [];
  const moves: Array<{ id: ClipId; track: TrackId; start: number }> = [];
  for (const c of clips) {
    const p = preview.get(c.id);
    if (!p) continue;
    const id = newId();
    if (p.track === c.track) {
      commands.push(cmd("Clip", { type: "Duplicate", id: c.id, new_id: id, start: p.start }));
    } else {
      commands.push(cmd("Clip", { type: "Duplicate", id: c.id, new_id: id, start: park(c) }));
      moves.push({ id, track: p.track, start: p.start });
    }
  }
  if (moves.length) commands.push(cmd("Clip", { type: "Move", moves }));
  const n = commands.length - (moves.length ? 1 : 0);
  return asCountedStep(countedStep("undo.clips.copy", n, "copy clip", "copy clips"), commands);
}

/** Commit a resize drag. */
export function boundsCommand(clips: ReadonlyArray<Clip>, preview: ReadonlyMap<ClipId, ClipBounds>): Command | null {
  const commands: Command[] = [];
  for (const c of clips) {
    const p = preview.get(c.id);
    if (!p || !changed(boundsOf(c), p)) continue;
    commands.push(cmd("Clip", { type: "SetBounds", id: c.id, start: p.start, length: p.length, offset: p.offset }));
  }
  return asCountedStep(countedStep("undo.clips.resize", commands.length, "resize clip", "resize clips"), commands);
}

/** Split every clip that strictly contains `at`. */
export function splitCommand(clips: ReadonlyArray<Clip>, at: Beats, newId: () => string): Command | null {
  const commands = clips
    .filter((c) => at > startOf(c) + BEATS_EPSILON && at < endOf(c) - BEATS_EPSILON)
    .map((c) => cmd("Clip", { type: "Split", id: c.id, at, new_id: newId() }));
  return asCountedStep(countedStep("undo.clips.split", commands.length, "split clip", "split clips"), commands);
}

/** Duplicate the selection right after itself (the block keeps its internal spacing). */
export function duplicateCommand(clips: ReadonlyArray<Clip>, newId: () => string): Command | null {
  if (clips.length === 0) return null;
  const span = Math.max(...clips.map(endOf)) - Math.min(...clips.map(startOf));
  const commands = clips.map((c) => cmd("Clip", { type: "Duplicate", id: c.id, new_id: newId(), start: startOf(c) + span }));
  return asCountedStep(countedStep("undo.clips.duplicate", commands.length, "duplicate clip", "duplicate clips"), commands);
}

export function deleteCommand(ids: ReadonlyArray<ClipId>): Command | null {
  return ids.length ? cmd("Clip", { type: "Delete", ids: [...ids] }) : null;
}

/**
 * Toggle looping of the clips: if any is unlooped, loop them all (the loop region becomes
 * the clip's current content range, so nothing moves); otherwise unloop them all.
 */
export function toggleLoopCommand(clips: ReadonlyArray<Clip>): Command | null {
  if (clips.length === 0) return null;
  const enable = clips.some((c) => !c.looping.enabled);
  const commands = clips
    .filter((c) => c.looping.enabled !== enable)
    .map((c) =>
      cmd("Clip", {
        type: "SetLoop",
        id: c.id,
        looping: enable ? loopOfContent(c) : { ...c.looping, enabled: false },
      }),
    );
  return asCountedStep(
    enable
      ? countedStep("undo.clips.loop", commands.length, "loop clip", "loop clips")
      : countedStep("undo.clips.unloop", commands.length, "unloop clip", "unloop clips"),
    commands,
  );
}

/**
 * The loop region covering a clip's current content range. A leading gap (negative offset)
 * stays a gap: the loop starts at content 0 (loop bounds are never negative).
 */
function loopOfContent(c: Clip): Clip["looping"] {
  const start = Math.max(0, c.offset);
  const end = Math.max(c.offset + c.length, start + MIN_CLIP_LENGTH);
  return { enabled: true, start, end };
}
