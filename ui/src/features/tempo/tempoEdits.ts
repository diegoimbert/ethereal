/**
 * Tempo lane editing of several points (#621), pure: moving a selection, deleting, the
 * tempo clipboard (points or a time section) and pasting it. Every edit is ONE command (an
 * `Edit::Batch` of the existing tempo commands when it takes several), so one undo step,
 * and collab-safe. The controller's rules hold: the point at beat 0 never moves or goes, two
 * points never share a time, BPM stays in 20..=999.
 */

import type { Beats, Command, TempoCurve, TempoPoint } from "@/generated";
import { BEATS_EPSILON, beatsApproxEq } from "@/state/beats";
import { newId } from "@/transport";
import { addTempoPointCommand, clampBpm, editTempoPointCommand, isAtZero, removeTempoPointsCommand } from "./tempoCommands";
import { asCountedStep, countedStep, keyedStep, type StepLabel } from "@/features/arrangement/editMath";

/** One command for `commands` (none: null; one: itself; more: a labelled batch). */
export function batch(commands: Command[], label: StepLabel): Command | null {
  return asCountedStep(label, commands);
}

/** The points of `ids` that can be removed (never the one at beat 0). */
export function removableIds(points: ReadonlyArray<TempoPoint>, ids: Iterable<string>): string[] {
  const want = new Set(ids);
  return points.filter((p) => want.has(p.id) && !isAtZero(p)).map((p) => p.id);
}

/** Delete the selected points (the one at beat 0 stays), or null. */
export function deletePointsCommand(points: ReadonlyArray<TempoPoint>, ids: Iterable<string>): Command | null {
  const out = removableIds(points, ids);
  return out.length ? removeTempoPointsCommand(out) : null;
}

/** The points inside a time section `[start, end)`. */
export function pointsIn(points: ReadonlyArray<TempoPoint>, start: Beats, end: Beats): TempoPoint[] {
  return points.filter((p) => p.time >= start - BEATS_EPSILON && p.time < end - BEATS_EPSILON);
}

/** Delete a time section's points (the one at beat 0 stays), or null. */
export function deleteRangeCommand(points: ReadonlyArray<TempoPoint>, start: Beats, end: Beats): Command | null {
  return deletePointsCommand(
    points,
    pointsIn(points, start, end).map((p) => p.id),
  );
}

/**
 * Move `ids` by `dt` beats and `dbpm` BPM (the point at beat 0 keeps its time), as one
 * command, or null when the move is not allowed: a point would land at or before beat 0
 * (other than the one there) or on a point that doesn't move. Commands are ordered so no
 * intermediate step puts two moved points at one time.
 */
export function movePointsCommand(points: ReadonlyArray<TempoPoint>, ids: ReadonlySet<string>, dt: Beats, dbpm: number): Command | null {
  const moving = points.filter((p) => ids.has(p.id));
  if (moving.length === 0) return null;
  const still = points.filter((p) => !ids.has(p.id));
  const target = (p: TempoPoint) => (isAtZero(p) ? p.time : p.time + dt);
  for (const p of moving) {
    const t = target(p);
    if (!isAtZero(p) && t <= BEATS_EPSILON) return null;
    if (still.some((s) => beatsApproxEq(s.time, t))) return null;
  }
  // Moving right: the rightmost first; left: the leftmost first.
  const order = [...moving].sort((a, b) => (dt > 0 ? b.time - a.time : a.time - b.time));
  const commands = order.flatMap((p) => {
    const time = target(p);
    const bpm = clampBpm(Math.round((p.bpm + dbpm) * 100) / 100);
    if (beatsApproxEq(time, p.time) && bpm === p.bpm) return [];
    return [editTempoPointCommand(p.id, { time: beatsApproxEq(time, p.time) ? undefined : time, bpm: bpm !== p.bpm ? bpm : undefined })];
  });
  return batch(commands, countedStep("undo.tempo.move_points", moving.length, "move tempo point", "move tempo points"));
}

/** Set the curve of the selected points (ramp to the next point or hold). */
export function setCurvesCommand(points: ReadonlyArray<TempoPoint>, ids: Iterable<string>, curve: TempoCurve): Command | null {
  const want = new Set(ids);
  return batch(
    points.filter((p) => want.has(p.id) && p.curve !== curve).map((p) => editTempoPointCommand(p.id, { curve })),
    curve === "Linear" ? keyedStep("undo.tempo.ramp", "ramp tempo") : keyedStep("undo.tempo.hold", "hold tempo"),
  );
}

// ---- Clipboard ------------------------------------------------------------------------------

export interface TempoClip {
  /** Points relative to the copy's start. */
  points: ReadonlyArray<{ dt: Beats; bpm: number; curve: TempoCurve }>;
  /** Length of what was copied: a section's length, else the copied points' span. */
  span: Beats;
  /** A time section (pasting one selects where it landed). */
  range: boolean;
}

let clipboard: TempoClip | null = null;

export const tempoClipboard = (): TempoClip | null => clipboard;
export const hasTempoClipboard = (): boolean => clipboard !== null;
export function setTempoClipboard(clip: TempoClip | null): void {
  clipboard = clip;
}

/** The selected points as a clip (relative to the first), or null. */
export function clipOfPoints(points: ReadonlyArray<TempoPoint>, ids: ReadonlySet<string>): TempoClip | null {
  const sel = points.filter((p) => ids.has(p.id)).sort((a, b) => a.time - b.time);
  if (sel.length === 0) return null;
  const t0 = sel[0]!.time;
  return { points: sel.map((p) => ({ dt: p.time - t0, bpm: p.bpm, curve: p.curve })), span: sel[sel.length - 1]!.time - t0, range: false };
}

/**
 * A time section as a clip. The tempo in effect at its start is always part of it (a point
 * at the start, copied from the point before when there is none there), so the section pastes
 * with the tempo it had.
 */
export function clipOfRange(points: ReadonlyArray<TempoPoint>, start: Beats, end: Beats): TempoClip | null {
  if (end - start <= BEATS_EPSILON) return null;
  const inside = pointsIn(points, start, end);
  const before = [...points].filter((p) => p.time < start - BEATS_EPSILON).sort((a, b) => b.time - a.time)[0];
  const head = inside[0] && beatsApproxEq(inside[0].time, start) ? [] : before ? [{ dt: 0, bpm: before.bpm, curve: "Step" as TempoCurve }] : [];
  return { points: [...head, ...inside.map((p) => ({ dt: p.time - start, bpm: p.bpm, curve: p.curve }))], span: end - start, range: true };
}

/**
 * Paste `clip` at `at`: each point lands at `at + dt`, taking over a point already there
 * (its BPM and curve change; the one at beat 0 too). With a section, the points already
 * inside the pasted span go first, so the span reads exactly as copied. One command, plus
 * the ids of the pasted points (to select them).
 */
export function pasteCommand(
  points: ReadonlyArray<TempoPoint>,
  clip: TempoClip,
  at: Beats,
  mint: () => string = newId,
): { command: Command | null; ids: string[] } {
  const base = Math.max(0, at);
  const commands: Command[] = [];
  const ids: string[] = [];
  const landing = clip.points.map((p) => base + p.dt);
  /** A point already at `t` (it takes the pasted BPM and curve). */
  const at_ = (t: Beats) => points.find((x) => beatsApproxEq(x.time, t));
  if (clip.range) {
    // The section's span is replaced: what is in it and not landed on goes.
    const inSpan = pointsIn(points, base, base + clip.span).filter((p) => !landing.some((t) => beatsApproxEq(t, p.time)));
    const removed = removableIds(
      points,
      inSpan.map((p) => p.id),
    );
    if (removed.length) commands.push(removeTempoPointsCommand(removed));
  }
  clip.points.forEach((p, i) => {
    const time = landing[i]!;
    const there = at_(time);
    if (there) {
      commands.push(editTempoPointCommand(there.id, { bpm: p.bpm, curve: p.curve }));
      ids.push(there.id);
    } else {
      const id = mint();
      commands.push(addTempoPointCommand(time, p.bpm, p.curve, id));
      ids.push(id);
    }
  });
  return { command: batch(commands, keyedStep("undo.tempo.paste", "paste tempo")), ids };
}
