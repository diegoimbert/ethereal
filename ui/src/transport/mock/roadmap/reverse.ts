/**
 * Mock of `NoteCommand::Reverse` (Rust: `ether-controller/src/doc/reverse.rs`, which has the
 * semantics). Mirrors notes, per-note expressions and expression lanes; clip envelopes are
 * left alone here (the engine mirrors them).
 */

import type { ClipId, CurveShape, ExpressionPoint, NoteId } from "@/generated";
import { BEATS_EPSILON } from "@/state/beats";
import { fail, type ReducerContext } from "../documentReducer";

interface Pt {
  t: number;
  v: number;
  curve: CurveShape;
}

const mirrorShape = (c: CurveShape): CurveShape => (c.type === "Curve" ? { type: "Curve", tension: -c.tension } : c);

function valueAt(pts: Pt[], t: number): [number, CurveShape] {
  let i = pts.findIndex((p) => p.t > t);
  if (i < 0) i = pts.length;
  if (i === 0) return [pts[0]!.v, pts[0]!.curve];
  if (i >= pts.length) return [pts[pts.length - 1]!.v, { type: "Step" }];
  const a = pts[i - 1]!;
  const b = pts[i]!;
  const shape: CurveShape = a.curve.type === "Curve" ? { type: "Linear" } : a.curve;
  const x = b.t > a.t ? (t - a.t) / (b.t - a.t) : 1;
  const frac = a.curve.type === "Step" ? 0 : a.curve.type === "Linear" ? x : x ** (4 ** Math.max(-1, Math.min(1, a.curve.tension)));
  return [a.v + (b.v - a.v) * frac, shape];
}

/** `pts` with the signal inside `[r0, r1]` mirrored in time (the Rust `reverse_points`). */
export function reversePoints(pts: Pt[], r0: number, r1: number): Pt[] {
  const left = pts.filter((p) => p.t < r0);
  const inner = pts.filter((p) => p.t >= r0 && p.t <= r1);
  const right = pts.filter((p) => p.t > r1);
  if (inner.length === 0 || r1 <= r0) return pts;
  const trail = inner[inner.length - 1]!.curve;
  const v: Pt[] = [];
  let front = false;
  let back = false;
  if (inner[0]!.t > r0) {
    front = true;
    const [val, curve]: [number, CurveShape] = left.length === 0 ? [inner[0]!.v, { type: "Step" }] : valueAt(pts, r0);
    v.push({ t: r0, v: val, curve });
  }
  v.push(...inner);
  const shapes = v.map((p) => p.curve);
  if (inner[inner.length - 1]!.t < r1) {
    back = true;
    const last = inner[inner.length - 1]!;
    const [val, shape]: [number, CurveShape] =
      right.length === 0 ? [last.v, { type: "Step" }] : [valueAt(pts, r1)[0], trail.type === "Curve" ? { type: "Linear" } : trail];
    shapes[shapes.length - 1] = shape;
    v.push({ t: r1, v: val, curve: { type: "Step" } });
    shapes.push({ type: "Step" });
  }
  const n = v.length - 1;
  const m = (t: number) => r0 + r1 - t;
  const rev: Pt[] = [];
  for (let k = n - 1; k >= 0; k--) {
    const s = shapes[k]!;
    rev.push({ t: m(v[k + 1]!.t), v: s.type === "Step" ? v[k]!.v : v[k + 1]!.v, curve: mirrorShape(s) });
  }
  rev.push({ t: m(v[0]!.t), v: v[0]!.v, curve: trail });
  const has = (t: number) => pts.some((p) => p.t === t);
  const flat = (a: number, b: number) => Math.abs(a - b) <= 1e-9;
  if ((back || !has(r0)) && left.length === 0 && rev.length > 1 && flat(rev[0]!.v, rev[1]!.v)) rev.shift();
  if ((front || !has(r1)) && right.length === 0 && rev.length > 1 && flat(rev[rev.length - 1]!.v, rev[rev.length - 2]!.v)) rev.pop();
  if (right.length === 0 && rev.length) rev[rev.length - 1]!.curve = trail;
  return [...left, ...rev, ...right];
}

const toPts = (ps: ReadonlyArray<ExpressionPoint>): Pt[] => ps.map((p) => ({ t: p.time, v: p.value, curve: p.curve }));
const fromPts = (ps: Pt[]): ExpressionPoint[] => ps.map((p) => ({ time: Math.max(0, p.t), value: p.v, curve: p.curve }));
const same = (a: ReadonlyArray<ExpressionPoint>, b: ReadonlyArray<ExpressionPoint>) => JSON.stringify(a) === JSON.stringify(b);

export function reverseNotes(ctx: ReducerContext, clipId: ClipId, ids: ReadonlyArray<NoteId> | null): void {
  const { tx } = ctx;
  const cl = tx.get("Clip", clipId) ?? fail("NotFound", `clip ${clipId}`);
  if (cl.content.type !== "Midi") fail("InvalidArgument", `clip ${clipId} is not a MIDI clip`);
  let targets = tx.all("Note").filter((n) => n.clip === clipId);
  let r0: number;
  let r1: number;
  if (ids === null) {
    [r0, r1] = cl.looping.enabled ? [cl.looping.start, cl.looping.end] : [Math.max(cl.offset, 0), cl.offset + cl.length];
  } else {
    const set = new Set(ids);
    targets = targets.filter((n) => set.has(n.id));
    r0 = Math.min(...targets.map((n) => n.start));
    r1 = Math.max(...targets.map((n) => n.start + n.duration));
  }
  if (!(Number.isFinite(r0) && Number.isFinite(r1) && r1 > r0)) return;
  for (const n of targets) {
    const end = n.start + n.duration;
    if (n.start < r0 - BEATS_EPSILON || end > r1 + BEATS_EPSILON) continue;
    const start = Math.max(0, r0 + r1 - end);
    if (Math.abs(start - n.start) > BEATS_EPSILON) tx.upsert("Note", { ...n, start });
    for (const e of tx.all("NoteExpression")) {
      if (e.note !== n.id) continue;
      const points = fromPts(reversePoints(toPts(e.points), 0, n.duration));
      if (!same(points, e.points)) tx.upsert("NoteExpression", { ...e, points });
    }
  }
  if (ids === null) {
    for (const l of tx.all("ExpressionLane")) {
      if (l.clip !== clipId) continue;
      const points = fromPts(reversePoints(toPts(l.points), r0, r1));
      if (!same(points, l.points)) tx.upsert("ExpressionLane", { ...l, points });
    }
  }
}
