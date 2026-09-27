/**
 * Mock of `TimeEdit::*` (v0.2, `time-edits`; CONTRACTS.md §12.3). Mirrors the controller
 * (`crates/ether-controller/src/time_edit/`): split across tracks, cut/copy/paste,
 * delete/insert/duplicate time, one undo step each; `Copy` fills a mock time clipboard.
 * Differences: new ids come from the mock's id generator (not `derive_id(seed, i)`), and the
 * replay check is skipped.
 */

import type {
  AutomationLane,
  AutomationPoint,
  AutomationTarget,
  Clip,
  CompRegion,
  CurveShape,
  Note,
  Project,
  ReplyValue,
  TimeEditCommand,
  TimeSelection,
  Track,
  TrackId,
  WarpMarker,
} from "@/generated";
import { fail } from "../documentReducer";
import type { Tx } from "../tx";
import type { MockHost } from "./host";

const EPS = 1e-6;
const TIME_EPS = 1e-9;
const UNIT: ReplyValue = { type: "Unit" };

export interface TimeEditHost extends MockHost {
  /** Run `edit` as one undo step (emits the patch). */
  transact(label: string, edit: (tx: Tx) => void): void;
}

// ─── Automation points (pure; see the controller's `points.rs`) ──────────────────────────

interface Pt {
  id: string | null;
  time: number;
  value: number;
  curve: CurveShape;
}

const pt = (time: number, value: number, curve: CurveShape): Pt => ({
  id: null,
  time,
  value,
  curve,
});

function shape(curve: CurveShape, x: number): number {
  if (curve.type === "Linear") return x;
  if (curve.type === "Step") return 0;
  const t = Math.max(-1, Math.min(1, curve.tension));
  return Math.pow(Math.max(0, Math.min(1, x)), Math.pow(4, t));
}

function segment(p0: Pt, p1: Pt, time: number): number {
  const span = p1.time - p0.time;
  return span <= 0 ? p1.value : p0.value + (p1.value - p0.value) * shape(p0.curve, (time - p0.time) / span);
}

/** Value at `time` (right limit). */
export function valueAt(pts: Pt[], time: number): number | null {
  if (!pts.length) return null;
  if (time < pts[0]!.time) return pts[0]!.value;
  const i = pts.findIndex((p) => p.time > time);
  return i < 0 ? pts.at(-1)!.value : segment(pts[i - 1]!, pts[i]!, time);
}

/** Value just before `time` (left limit). */
function valueBefore(pts: Pt[], time: number): number | null {
  if (!pts.length) return null;
  if (time <= pts[0]!.time) return pts[0]!.value;
  const i = pts.findIndex((p) => p.time >= time);
  return i < 0 ? pts.at(-1)!.value : segment(pts[i - 1]!, pts[i]!, time);
}

function curveAt(pts: Pt[], time: number): CurveShape {
  let c: CurveShape = { type: "Linear" };
  for (const p of pts) if (p.time <= time) c = p.curve;
  return c;
}

const shifted = (p: Pt, d: number): Pt => ({ ...p, time: p.time + d });

function deletePoints(pts: Pt[], a: number, b: number): Pt[] {
  const len = b - a;
  const hasIn = pts.some((p) => p.time >= a - TIME_EPS && p.time <= b + TIME_EPS);
  const before = pts.some((p) => p.time < a - TIME_EPS);
  const after = pts.some((p) => p.time > b + TIME_EPS);
  if (!hasIn && !(before && after)) return pts.map((p) => (p.time > b ? shifted(p, -len) : p));
  const left = valueBefore(pts, a)!;
  const right = valueAt(pts, b)!;
  const rc = curveAt(pts, b);
  const out = pts.filter((p) => p.time < a - TIME_EPS);
  if (Math.abs(left - right) <= TIME_EPS) out.push(pt(a, left, rc));
  else out.push(pt(a, left, { type: "Linear" }), pt(a, right, rc));
  return [...out, ...pts.filter((p) => p.time > b + TIME_EPS).map((p) => shifted(p, -len))];
}

function insertPoints(pts: Pt[], at: number, len: number): Pt[] {
  const move = (p: Pt) => (p.time >= at - TIME_EPS ? shifted(p, len) : p);
  const before = pts.some((p) => p.time < at - TIME_EPS);
  const after = pts.some((p) => p.time >= at - TIME_EPS);
  if (!(before && after)) return pts.map(move);
  const out = pts.filter((p) => p.time < at - TIME_EPS);
  out.push(pt(at, valueBefore(pts, at)!, { type: "Step" }));
  if (!pts.some((p) => Math.abs(p.time - at) <= TIME_EPS)) out.push(pt(at + len, valueAt(pts, at)!, curveAt(pts, at)));
  return [...out, ...pts.filter((p) => p.time >= at - TIME_EPS).map(move)];
}

function slicePoints(pts: Pt[], a: number, b: number): Pt[] | null {
  if (!pts.length) return null;
  return [
    pt(0, valueAt(pts, a)!, curveAt(pts, a)),
    ...pts.filter((p) => p.time > a + TIME_EPS && p.time < b - TIME_EPS).map((p) => ({ ...p, id: null, time: p.time - a })),
    pt(b - a, valueBefore(pts, b)!, { type: "Linear" }),
  ];
}

function overwritePoints(pts: Pt[], at: number, piece: Pt[], len: number): Pt[] {
  const end = at + len;
  const out = pts.filter((p) => p.time < at - TIME_EPS);
  if (out.length) out.push(pt(at, valueBefore(pts, at)!, { type: "Linear" }));
  out.push(...piece.map((p) => ({ ...p, id: null, time: p.time + at })));
  if (pts.some((p) => p.time > end + TIME_EPS)) out.push(pt(end, valueAt(pts, end)!, curveAt(pts, end)));
  return [...out, ...pts.filter((p) => p.time > end + TIME_EPS)];
}

// ─── Clipboard ──────────────────────────────────────────────────────────────────────────

interface ClipBundle {
  clip: Clip;
  notes: Note[];
  warp: WarpMarker[];
  envelopes: Array<{ lane: AutomationLane; points: AutomationPoint[] }>;
}

interface TrackCopy {
  track: TrackId;
  kind: Track["kind"];
  clips: ClipBundle[];
  regions: CompRegion[];
  lanes: Array<{ target: AutomationTarget; enabled: boolean; points: Pt[] }>;
}

interface TimeClipboard {
  length: number;
  tracks: TrackCopy[];
}

const isContent = (t: Track) => t.kind === "Audio" || t.kind === "Midi" || t.kind === "Group";
const byStart = (a: { start: number; id: string }, b: { start: number; id: string }) => a.start - b.start || (a.id < b.id ? -1 : 1);

/** Every track in display order (depth first). */
function displayOrder(p: Project): Track[] {
  const all = Object.values(p.tracks);
  const children = (parent: TrackId | null) =>
    all.filter((t) => t.parent === parent).sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : a.id < b.id ? -1 : 1));
  const out: Track[] = [];
  const walk = (t: Track) => {
    out.push(t);
    children(t.id).forEach(walk);
  };
  children(null).forEach(walk);
  return out;
}

function frozenCheck(tracks: Track[]): void {
  const frozen = tracks.find((t) => t.freeze);
  if (frozen) fail("InvalidState", `track "${frozen.name}" is frozen: unfreeze it to edit its time line`);
}

/** The tracks a time edit applies to (display order). */
function scope(p: Project, tracks: TrackId[], global: boolean): Track[] {
  const order = displayOrder(p);
  const wanted = new Set<TrackId>();
  if (!tracks.length) order.filter(isContent).forEach((t) => wanted.add(t.id));
  for (const id of tracks) {
    if (!p.tracks[id]) fail("NotFound", `track ${id} not found`);
    const stack = [id];
    while (stack.length) {
      const t = stack.pop()!;
      if (wanted.has(t)) continue;
      wanted.add(t);
      stack.push(...order.filter((c) => c.parent === t).map((c) => c.id));
    }
  }
  if (global) {
    if (!order.filter(isContent).every((t) => wanted.has(t.id))) {
      fail("InvalidArgument", "a global time edit must cover every track (the whole song)");
    }
    order.forEach((t) => wanted.add(t.id));
  }
  const out = order.filter((t) => wanted.has(t.id));
  frozenCheck(out);
  return out;
}

function range(s: TimeSelection): [number, number] {
  if (!(s.start >= 0 && s.end >= 0)) fail("InvalidArgument", "the time selection must be >= 0");
  if (s.end - s.start <= EPS) fail("InvalidArgument", "the time selection is empty");
  return [s.start, s.end];
}

export class MockTimeEdits {
  private clipboard: TimeClipboard | null = null;

  constructor(private readonly host: TimeEditHost) {}

  command(c: TimeEditCommand): ReplyValue {
    const p = this.host.project();
    switch (c.type) {
      case "Copy": {
        const [a, b] = range(c.selection);
        this.clipboard = this.copy(p, scope(p, c.selection.tracks, false).filter(isContent), a, b);
        return UNIT;
      }
      case "Cut": {
        const [a, b] = range(c.selection);
        const cb = this.copy(p, scope(p, c.selection.tracks, c.selection.global).filter(isContent), a, b);
        this.host.transact("Cut Time", (tx) => this.deleteTime(tx, c.selection));
        this.clipboard = cb;
        return UNIT;
      }
      case "Split":
        this.host.transact("Split", (tx) => {
          for (const t of scope(tx.project, c.tracks, false)) this.split(tx, t.id, c.at);
        });
        return UNIT;
      case "DeleteTime":
        this.host.transact("Delete Time", (tx) => this.deleteTime(tx, c.selection));
        return UNIT;
      case "InsertSilence":
        if (!(c.at >= 0)) fail("InvalidArgument", "the insert position must be >= 0");
        if (!(c.length > EPS)) fail("InvalidArgument", "the inserted length must be > 0");
        this.host.transact("Insert Silence", (tx) => this.insertSilence(tx, c.tracks, c.at, c.length, c.global));
        return UNIT;
      case "Paste": {
        const cb = this.clipboard;
        if (!cb) fail("InvalidState", "the time clipboard is empty");
        const pairs: Array<[number, TrackId]> = [];
        cb.tracks.forEach((src, i) => {
          const dest = c.tracks.length ? c.tracks[i] : src.track;
          if (dest === undefined) return;
          const t = p.tracks[dest];
          if (!t && c.tracks.length) fail("NotFound", `track ${dest} not found`);
          if (t && t.kind === src.kind && !pairs.some(([, d]) => d === dest)) pairs.push([i, dest]);
        });
        frozenCheck(pairs.map(([, d]) => p.tracks[d]!));
        if (!pairs.length) fail("InvalidArgument", "none of the target tracks can take the copied material");
        this.host.transact(c.insert ? "Paste Time" : "Paste", (tx) => this.paste(tx, cb, c.at, pairs, c.insert));
        return UNIT;
      }
      case "DuplicateTime": {
        const [a, b] = range(c.selection);
        const tracks = scope(p, c.selection.tracks, c.selection.global).filter(isContent);
        const cb = this.copy(p, tracks, a, b);
        this.host.transact("Duplicate Time", (tx) => {
          this.insertSilence(tx, c.selection.tracks, b, b - a, c.selection.global);
          this.paste(
            tx,
            cb,
            b,
            tracks.map((t, i) => [i, t.id]),
            false,
          );
        });
        return UNIT;
      }
    }
  }

  // ─── Per-track edits ──────────────────────────────────────────────────────────────────

  private clips(tx: Tx, track: TrackId): Clip[] {
    return tx
      .all("Clip")
      .filter((c) => c.track === track)
      .sort(byStart);
  }

  private bundle(tx: Tx, c: Clip): ClipBundle {
    const envelopes = tx
      .all("AutomationLane")
      .filter((l) => l.owner.type === "Clip" && l.owner.clip === c.id)
      .map((lane) => ({
        lane,
        points: tx.all("AutomationPoint").filter((p) => p.lane === lane.id),
      }));
    return {
      clip: c,
      notes: tx.all("Note").filter((n) => n.clip === c.id),
      warp: tx.all("WarpMarker").filter((m) => m.clip === c.id),
      envelopes,
    };
  }

  private insertBundle(tx: Tx, b: ClipBundle, clip: Clip, keepEnvelope: (t: AutomationTarget) => AutomationTarget | null): void {
    tx.upsert("Clip", clip);
    for (const n of b.notes) tx.upsert("Note", { ...n, id: this.host.newId(), clip: clip.id });
    for (const m of b.warp) tx.upsert("WarpMarker", { ...m, id: this.host.newId(), clip: clip.id });
    for (const { lane, points } of b.envelopes) {
      const target = keepEnvelope(lane.target);
      if (!target) continue;
      const id = this.host.newId();
      tx.upsert("AutomationLane", {
        ...lane,
        id,
        owner: { type: "Clip", clip: clip.id },
        target,
      });
      for (const p of points) tx.upsert("AutomationPoint", { ...p, id: this.host.newId(), lane: id });
    }
  }

  private split(tx: Tx, track: TrackId, at: number): void {
    for (const c of this.clips(tx, track)) {
      const [s, e] = [c.start, c.start + c.length];
      if (!(at > s + EPS && at < e - EPS)) continue;
      this.insertBundle(tx, this.bundle(tx, c), { ...piece(c, at, e), id: this.host.newId() }, (t) => t);
      tx.upsert("Clip", piece(c, s, at));
    }
  }

  private clearClips(tx: Tx, track: TrackId, a: number, b: number): void {
    this.split(tx, track, a);
    this.split(tx, track, b);
    for (const c of this.clips(tx, track)) {
      if (c.start >= a - EPS && c.start + c.length <= b + EPS) deleteClip(tx, c.id);
    }
  }

  private shiftClips(tx: Tx, track: TrackId, from: number, delta: number): void {
    for (const c of this.clips(tx, track)) if (c.start >= from - EPS) tx.upsert("Clip", { ...c, start: Math.max(0, c.start + delta) });
  }

  private regions(tx: Tx, track: TrackId): CompRegion[] {
    return tx
      .all("CompRegion")
      .filter((r) => r.track === track)
      .sort(byStart);
  }

  private editLanes(tx: Tx, track: TrackId, f: (pts: Pt[]) => Pt[]): void {
    for (const lane of tx.all("AutomationLane")) {
      if (lane.owner.type !== "Track" || lane.owner.track !== track) continue;
      const pts = lanePoints(tx, lane.id);
      if (pts.length) this.rewriteLane(tx, lane.id, f(pts));
    }
  }

  private rewriteLane(tx: Tx, lane: string, pts: Pt[]): void {
    for (const p of tx.all("AutomationPoint")) if (p.lane === lane) tx.remove("AutomationPoint", p.id);
    // (Unlike the controller, same-time jumps are not ordered by id here.)
    for (const p of pts)
      tx.upsert("AutomationPoint", {
        id: this.host.newId(),
        lane,
        time: Math.max(0, p.time),
        value: p.value,
        curve: p.curve,
      });
  }

  private deleteTrackTime(tx: Tx, track: TrackId, a: number, b: number): void {
    const len = b - a;
    this.clearClips(tx, track, a, b);
    this.shiftClips(tx, track, b, -len);
    for (const r of this.regions(tx, track)) {
      if (r.end <= a + EPS) continue;
      const [s, e] = r.start >= b - EPS ? [r.start - len, r.end - len] : [Math.min(r.start, a), r.end <= b ? a : r.end - len];
      if (e - s <= EPS) tx.remove("CompRegion", r.id);
      else tx.upsert("CompRegion", { ...r, start: s, end: e });
    }
    this.editLanes(tx, track, (pts) => deletePoints(pts, a, b));
  }

  private insertTrackTime(tx: Tx, track: TrackId, at: number, len: number): void {
    this.split(tx, track, at);
    this.shiftClips(tx, track, at, len);
    for (const r of this.regions(tx, track)) {
      if (r.start >= at - EPS)
        tx.upsert("CompRegion", {
          ...r,
          start: r.start + len,
          end: r.end + len,
        });
      else if (r.end > at + EPS) {
        tx.upsert("CompRegion", { ...r, end: at });
        tx.upsert("CompRegion", {
          ...r,
          id: this.host.newId(),
          start: at + len,
          end: r.end + len,
        });
      }
    }
    this.editLanes(tx, track, (pts) => insertPoints(pts, at, len));
  }

  private clearRange(tx: Tx, track: TrackId, a: number, b: number): void {
    this.clearClips(tx, track, a, b);
    for (const r of this.regions(tx, track)) {
      if (r.end <= a + EPS || r.start >= b - EPS) continue;
      const left = r.start < a - EPS;
      const right = r.end > b + EPS;
      if (!left && !right) tx.remove("CompRegion", r.id);
      else if (left && !right) tx.upsert("CompRegion", { ...r, end: a });
      else if (!left && right) tx.upsert("CompRegion", { ...r, start: b });
      else {
        tx.upsert("CompRegion", { ...r, end: a });
        tx.upsert("CompRegion", { ...r, id: this.host.newId(), start: b });
      }
    }
  }

  private deleteTime(tx: Tx, sel: TimeSelection): void {
    const [a, b] = range(sel);
    for (const t of scope(tx.project, sel.tracks, sel.global)) this.deleteTrackTime(tx, t.id, a, b);
    if (!sel.global) return;
    const len = b - a;
    for (const m of tx.all("Marker")) {
      if (m.position > a + EPS && m.position < b - EPS) tx.remove("Marker", m.id);
      else if (m.position >= b - EPS) tx.upsert("Marker", { ...m, position: m.position - len });
    }
    for (const p of tx.all("TempoPoint")) {
      if (p.time <= EPS) continue;
      if (p.time >= a - EPS && p.time < b - EPS) tx.remove("TempoPoint", p.id);
      else if (p.time >= b - EPS) tx.upsert("TempoPoint", { ...p, time: p.time - len });
    }
    for (const s of tx.all("TimeSignature")) {
      if (s.time <= EPS) continue;
      if (s.time >= a - EPS && s.time < b - EPS) tx.remove("TimeSignature", s.id);
      else if (s.time >= b - EPS) tx.upsert("TimeSignature", { ...s, time: s.time - len });
    }
    this.moveLoop(tx, (x) => (x < a ? x : x < b ? a : x - len));
  }

  private insertSilence(tx: Tx, tracks: TrackId[], at: number, len: number, global: boolean): void {
    for (const t of scope(tx.project, tracks, global)) this.insertTrackTime(tx, t.id, at, len);
    if (!global) return;
    for (const m of tx.all("Marker")) if (m.position >= at - EPS) tx.upsert("Marker", { ...m, position: m.position + len });
    for (const p of tx.all("TempoPoint")) if (p.time > EPS && p.time >= at - EPS) tx.upsert("TempoPoint", { ...p, time: p.time + len });
    for (const s of tx.all("TimeSignature"))
      if (s.time > EPS && s.time >= at - EPS) tx.upsert("TimeSignature", { ...s, time: s.time + len });
    this.moveLoop(tx, (x) => (x < at ? x : x + len));
  }

  private moveLoop(tx: Tx, f: (x: number) => number): void {
    const r = tx.project.settings.loop_region;
    const [start, end] = [f(r.start), f(r.end)];
    if (end - start > EPS && (start !== r.start || end !== r.end)) tx.setSettings({ ...tx.project.settings, loop_region: { start, end } });
  }

  // ─── Clipboard ────────────────────────────────────────────────────────────────────────

  private copy(p: Project, tracks: Track[], a: number, b: number): TimeClipboard {
    const clips = Object.values(p.clips);
    const out: TrackCopy[] = tracks.map((t) => ({
      track: t.id,
      kind: t.kind,
      clips: clips
        .filter((c) => c.track === t.id && c.start < b - EPS && c.start + c.length > a + EPS)
        .sort(byStart)
        .map((c) => {
          const cut = piece(c, Math.max(c.start, a), Math.min(c.start + c.length, b));
          return {
            clip: { ...cut, start: cut.start - a },
            notes: Object.values(p.notes).filter((n) => n.clip === c.id),
            warp: Object.values(p.warp_markers).filter((m) => m.clip === c.id),
            envelopes: Object.values(p.automation_lanes)
              .filter((l) => l.owner.type === "Clip" && l.owner.clip === c.id)
              .map((lane) => ({
                lane,
                points: Object.values(p.automation_points).filter((x) => x.lane === lane.id),
              })),
          };
        }),
      regions: Object.values(p.comp_regions)
        .filter((r) => r.track === t.id && r.start < b - EPS && r.end > a + EPS)
        .map((r) => ({
          ...r,
          start: Math.max(r.start, a) - a,
          end: Math.min(r.end, b) - a,
        })),
      lanes: Object.values(p.automation_lanes)
        .filter((l) => l.owner.type === "Track" && l.owner.track === t.id)
        .flatMap((l) => {
          const pts = slicePoints(projectPoints(p, l.id), a, b);
          return pts ? [{ target: l.target, enabled: l.enabled, points: pts }] : [];
        }),
    }));
    return { length: b - a, tracks: out };
  }

  private paste(tx: Tx, cb: TimeClipboard, at: number, pairs: Array<[number, TrackId]>, insert: boolean): void {
    const len = cb.length;
    for (const [, dest] of pairs) {
      if (insert) this.insertTrackTime(tx, dest, at, len);
      else this.clearRange(tx, dest, at, at + len);
    }
    for (const [i, dest] of pairs) {
      const src = cb.tracks[i]!;
      const laneOk = (lane: string) => src.track === dest && tx.get("TakeLane", lane)?.track === dest;
      const map = (t: AutomationTarget) => retarget(tx.project, src.track, dest, t);
      for (const b of src.clips) {
        if (b.clip.lane && !laneOk(b.clip.lane)) continue;
        this.insertBundle(
          tx,
          b,
          {
            ...b.clip,
            id: this.host.newId(),
            track: dest,
            start: b.clip.start + at,
          },
          map,
        );
      }
      for (const r of src.regions) {
        if (laneOk(r.lane))
          tx.upsert("CompRegion", {
            ...r,
            id: this.host.newId(),
            track: dest,
            start: r.start + at,
            end: r.end + at,
          });
      }
      for (const l of src.lanes) {
        const target = map(l.target);
        if (!target) continue;
        let lane = tx
          .all("AutomationLane")
          .find((x) => x.owner.type === "Track" && x.owner.track === dest && JSON.stringify(x.target) === JSON.stringify(target));
        if (!lane) {
          lane = {
            id: this.host.newId(),
            owner: { type: "Track", track: dest },
            target,
            enabled: l.enabled,
          };
          tx.upsert("AutomationLane", lane);
        }
        this.rewriteLane(tx, lane.id, overwritePoints(lanePoints(tx, lane.id), at, l.points, len));
      }
    }
  }
}

/** `c` restricted to `[s, e)`; fades only on the clip's own edges. */
function piece(c: Clip, s: number, e: number): Clip {
  const len = e - s;
  const out: Clip = {
    ...c,
    start: s,
    length: len,
    offset: c.offset + (s - c.start),
  };
  if (c.content.type === "Audio") {
    out.content = {
      ...c.content,
      fade_in: Math.abs(s - c.start) <= EPS ? Math.min(c.content.fade_in, len) : 0,
      fade_out: Math.abs(e - (c.start + c.length)) <= EPS ? Math.min(c.content.fade_out, len) : 0,
    };
  }
  return out;
}

function deleteClip(tx: Tx, id: string): void {
  for (const n of tx.all("Note")) if (n.clip === id) tx.remove("Note", n.id);
  for (const m of tx.all("WarpMarker")) if (m.clip === id) tx.remove("WarpMarker", m.id);
  for (const l of tx.all("AutomationLane")) {
    if (l.owner.type !== "Clip" || l.owner.clip !== id) continue;
    for (const p of tx.all("AutomationPoint")) if (p.lane === l.id) tx.remove("AutomationPoint", p.id);
    tx.remove("AutomationLane", l.id);
  }
  tx.remove("Clip", id);
}

function toPts(points: AutomationPoint[]): Pt[] {
  return points
    .sort((a, b) => a.time - b.time || (a.id < b.id ? -1 : 1))
    .map((p) => ({ id: p.id, time: p.time, value: p.value, curve: p.curve }));
}

const lanePoints = (tx: Tx, lane: string) => toPts(tx.all("AutomationPoint").filter((p) => p.lane === lane));
const projectPoints = (p: Project, lane: string) => toPts(Object.values(p.automation_points).filter((x) => x.lane === lane));

/** A track's automation target moved to another track (see the controller's `retarget`). */
function retarget(p: Project, from: TrackId, to: TrackId, t: AutomationTarget): AutomationTarget | null {
  switch (t.type) {
    case "TrackVolume":
    case "TrackPan":
      return t.track === from ? { ...t, track: to } : p.tracks[t.track] ? t : null;
    case "SendLevel": {
      if (from === to) return p.sends[t.send] ? t : null;
      const ret = p.sends[t.send]?.to;
      const s = Object.values(p.sends).find((x) => x.from === to && x.to === ret);
      return s ? { type: "SendLevel", send: s.id } : null;
    }
    case "DeviceParam":
      return p.devices[t.device]?.track === to ? t : null;
  }
}
