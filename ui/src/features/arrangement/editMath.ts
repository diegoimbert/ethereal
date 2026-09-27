/**
 * Clip edit math and command building (pure): drag previews for move / resize-start /
 * resize-end, and the commands for move, copy, resize, split, duplicate, delete and loop.
 *
 * A drag previews locally and commits once, on release: one command (or one `Edit::Batch`)
 * per drag, so every drag is one undo step.
 */

import type { Beats, Clip, ClipId, Command, TrackId } from "@/generated";
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
    let d = snap(a + deltaBeats) - a;
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

  // resize-start: the end stays put, the content offset follows the start.
  const a = startOf(anchor);
  const d = snap(a + deltaBeats) - a;
  for (const c of clips) {
    const s0 = startOf(c);
    const end = s0 + c.length;
    let s = Math.min(Math.max(0, s0 + d), end - MIN_CLIP_LENGTH);
    if (c.offset + (s - s0) < 0) s = s0 - c.offset;
    out.set(c.id, { track: c.track, start: s, length: end - s, offset: c.offset + (s - s0) });
  }
  return out;
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
export function asOneStep(label: string, commands: Command[]): Command | null {
  if (commands.length === 0) return null;
  if (commands.length === 1) return commands[0]!;
  return cmd("Edit", { type: "Batch", label, commands });
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
  return asOneStep("Copy Clips", commands);
}

/** Commit a resize drag. */
export function boundsCommand(clips: ReadonlyArray<Clip>, preview: ReadonlyMap<ClipId, ClipBounds>): Command | null {
  const commands: Command[] = [];
  for (const c of clips) {
    const p = preview.get(c.id);
    if (!p || !changed(boundsOf(c), p)) continue;
    commands.push(cmd("Clip", { type: "SetBounds", id: c.id, start: p.start, length: p.length, offset: p.offset }));
  }
  return asOneStep("Resize Clips", commands);
}

/** Split every clip that strictly contains `at`. */
export function splitCommand(clips: ReadonlyArray<Clip>, at: Beats, newId: () => string): Command | null {
  const commands = clips
    .filter((c) => at > startOf(c) + BEATS_EPSILON && at < endOf(c) - BEATS_EPSILON)
    .map((c) => cmd("Clip", { type: "Split", id: c.id, at, new_id: newId() }));
  return asOneStep("Split Clips", commands);
}

/** Duplicate the selection right after itself (the block keeps its internal spacing). */
export function duplicateCommand(clips: ReadonlyArray<Clip>, newId: () => string): Command | null {
  if (clips.length === 0) return null;
  const span = Math.max(...clips.map(endOf)) - Math.min(...clips.map(startOf));
  const commands = clips.map((c) =>
    cmd("Clip", { type: "Duplicate", id: c.id, new_id: newId(), start: startOf(c) + span }),
  );
  return asOneStep("Duplicate Clips", commands);
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
        looping: enable ? { enabled: true, start: c.offset, end: c.offset + c.length } : { ...c.looping, enabled: false },
      }),
    );
  return asOneStep(enable ? "Loop Clips" : "Unloop Clips", commands);
}
