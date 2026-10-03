/**
 * MIDI expression data model (v0.3, `midi-expression`; Rust `ether_model::expression`,
 * CONTRACTS.md §13.2). Pure functions shared by the piano-roll lanes and the MockTransport
 * (`transport/mock/roadmap/expression.ts`), so the mock validates exactly like the engine.
 *
 * - An `ExpressionLane` is one channel-wide curve of a MIDI clip: `Cc { controller }`
 *   (0..=119), `PitchBend` or `ChannelPressure`. At most one lane per `(clip, kind)`. Times
 *   are content-relative beats, like notes.
 * - A `NoteExpression` is one per-note curve (`Pitch`, `Pressure`, `Timbre`), at most one
 *   per `(note, kind)`. Times are beats from the note start.
 * - A curve is ONE value (`points`, sorted by time): edits replace a whole curve
 *   (`SetPoints`, `SetNoteExpression`) or a time range of it (`ReplaceRange`).
 *
 * Values: CC / pressure / timbre 0..1, pitch bend -1..1, note pitch ±96 semitones. Curves
 * interpolate with the point's `CurveShape` (the automation formula), and hold the first /
 * last value outside their points.
 */

import type { CurveShape, ExpressionKind, ExpressionPoint, NoteExpressionKind } from "@/generated";

/** Most points in one curve. */
export const MAX_EXPRESSION_POINTS = 16_384;
/** Highest CC of a lane (120..=127 are channel mode messages). */
export const MAX_EXPRESSION_CC = 119;
/** Largest per-note pitch offset, in semitones. */
export const MAX_NOTE_PITCH_OFFSET = 96;

export type Range = readonly [number, number];

export function expressionRange(kind: ExpressionKind): Range {
  return kind.type === "PitchBend" ? [-1, 1] : [0, 1];
}

export function noteExpressionRange(kind: NoteExpressionKind): Range {
  return kind === "Pitch" ? [-MAX_NOTE_PITCH_OFFSET, MAX_NOTE_PITCH_OFFSET] : [0, 1];
}

/** `ether_model::expression::check_points`: an error message, or `null` when valid. */
export function checkPoints(points: ReadonlyArray<ExpressionPoint>, range: Range): string | null {
  if (points.length > MAX_EXPRESSION_POINTS) return `an expression curve holds at most ${MAX_EXPRESSION_POINTS} points`;
  let last = 0;
  for (const p of points) {
    if (!(Number.isFinite(p.time) && p.time >= 0)) return "expression point times must be finite and >= 0";
    if (p.time < last) return "expression points must be sorted by time";
    last = p.time;
    if (!(Number.isFinite(p.value) && p.value >= range[0] && p.value <= range[1])) {
      return `expression value ${p.value} outside ${range[0]}..=${range[1]}`;
    }
    if (p.curve.type === "Curve" && !(Number.isFinite(p.curve.tension) && p.curve.tension >= -1 && p.curve.tension <= 1)) {
      return "curve tension must be in -1..=1";
    }
  }
  return null;
}

/**
 * `ExpressionCommand::ReplaceRange`: the points before `start`, then `points` (all inside
 * `[start, end)`), then the points at or after `end`. Returns an error message when the
 * range or the new points are invalid.
 */
export function replaceRange(
  old: ReadonlyArray<ExpressionPoint>,
  start: number,
  end: number,
  points: ReadonlyArray<ExpressionPoint>,
): ExpressionPoint[] | string {
  if (!(Number.isFinite(start) && Number.isFinite(end) && start >= 0 && start <= end)) return "invalid range";
  if (points.some((p) => !(p.time >= start && p.time < end))) return "replacement points must lie inside the range";
  return [...old.filter((p) => p.time < start), ...points, ...old.filter((p) => p.time >= end)];
}

/** Stable identity of a lane kind (one lane per `(clip, kind)`). */
export function kindKey(kind: ExpressionKind): string {
  return kind.type === "Cc" ? `cc${kind.controller}` : kind.type;
}

export function sameKind(a: ExpressionKind, b: ExpressionKind): boolean {
  return kindKey(a) === kindKey(b);
}

const CC_NAMES: Record<number, string> = {
  1: "Mod Wheel",
  2: "Breath",
  7: "Volume",
  10: "Pan",
  11: "Expression",
  64: "Sustain",
  74: "Brightness",
};

/** Lane label, e.g. "Pitch Bend", "CC 1 Mod Wheel", "CC 20". */
export function kindLabel(kind: ExpressionKind): string {
  switch (kind.type) {
    case "PitchBend":
      return "Pitch Bend";
    case "ChannelPressure":
      return "Channel Pressure";
    case "Cc": {
      const name = CC_NAMES[kind.controller];
      return name ? `CC ${kind.controller} ${name}` : `CC ${kind.controller}`;
    }
  }
}

export function noteKindLabel(kind: NoteExpressionKind): string {
  return kind === "Pitch" ? "Note Pitch" : kind === "Pressure" ? "Note Pressure" : "Note Timbre";
}

/** The value as the receiver sees it: MIDI 0..127, bend -8192..8191, semitones. */
export function formatValue(kind: ExpressionKind | NoteExpressionKind, value: number): string {
  if (typeof kind === "string") {
    return kind === "Pitch" ? `${value >= 0 ? "+" : ""}${value.toFixed(2)} st` : String(Math.round(value * 127));
  }
  if (kind.type === "PitchBend") return String(Math.round(value * 8191));
  return String(Math.round(value * 127));
}

const LINEAR: CurveShape = { type: "Linear" };

/**
 * Pencil stroke → the points that replace `[start, end)`: one point per sample (sorted,
 * values clamped to `range`), thinned so consecutive points are at least `minGap` beats
 * apart (the last sample is always kept so the stroke ends where the pointer left it).
 */
export function strokePoints(
  samples: ReadonlyArray<{ time: number; value: number }>,
  range: Range,
  minGap: number,
): { start: number; end: number; points: ExpressionPoint[] } | null {
  if (samples.length === 0) return null;
  const sorted = [...samples].map((s) => ({ time: Math.max(0, s.time), value: s.value })).sort((a, b) => a.time - b.time);
  const clampV = (v: number) => Math.min(range[1], Math.max(range[0], v));
  const points: ExpressionPoint[] = [];
  for (const s of sorted) {
    const prev = points[points.length - 1];
    if (prev && s.time - prev.time < minGap) continue;
    points.push({ time: s.time, value: clampV(s.value), curve: LINEAR });
  }
  const tail = sorted[sorted.length - 1]!;
  const last = points[points.length - 1]!;
  if (last.time !== tail.time) {
    if (points.length > 1 && tail.time - points[points.length - 2]!.time < minGap * 2) points.pop();
    points.push({ time: tail.time, value: clampV(tail.value), curve: LINEAR });
  }
  const start = points[0]!.time;
  // `end` is exclusive: just past the last point so it is replaced too.
  const end = points[points.length - 1]!.time + Math.max(minGap / 2, 1e-6);
  return { start, end, points };
}

/** Insert (or replace at the same time) one point, keeping the curve sorted. */
export function withPoint(points: ReadonlyArray<ExpressionPoint>, p: ExpressionPoint): ExpressionPoint[] {
  const out = points.filter((q) => q.time !== p.time);
  const i = out.findIndex((q) => q.time > p.time);
  out.splice(i < 0 ? out.length : i, 0, p);
  return out;
}

/** Move point `index` to (time, value), clamped between its neighbours so order holds. */
export function movePoint(
  points: ReadonlyArray<ExpressionPoint>,
  index: number,
  time: number,
  value: number,
  range: Range,
): ExpressionPoint[] {
  const lo = index > 0 ? points[index - 1]!.time : 0;
  const hi = index < points.length - 1 ? points[index + 1]!.time : Number.POSITIVE_INFINITY;
  const out = points.slice();
  out[index] = {
    ...points[index]!,
    time: Math.min(hi, Math.max(lo, time)),
    value: Math.min(range[1], Math.max(range[0], value)),
  };
  return out;
}
