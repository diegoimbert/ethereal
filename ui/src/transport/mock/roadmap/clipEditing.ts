/**
 * Mock of `Marker::*` and the v2 clip commands (`SetFadeCurves`, `SetReversed`,
 * `Crossfade`; `SetCrossfade` from `crossfades`; `clip-editing`).
 */

import type { Clip, ClipCommand, FadeCurve, MarkerCommand } from "@/generated";
import { BEATS_EPSILON } from "@/state/beats";
import { fail, type ReducerContext } from "../documentReducer";
import { bpmAt } from "../tempo";

function checkCurve(c: FadeCurve | null): void {
  if (c?.type === "Curve" && !(c.tension >= -1 && c.tension <= 1)) fail("InvalidArgument", "curve tension must be in -1..=1");
}

export function markerCommand(ctx: ReducerContext, c: MarkerCommand): void {
  const { tx } = ctx;
  const marker = (id: string) => tx.get("Marker", id) ?? fail("NotFound", `marker ${id}`);
  switch (c.type) {
    case "Add":
      if (tx.get("Marker", c.id)) return;
      if (!(c.position >= 0)) fail("InvalidArgument", "marker position must be >= 0");
      tx.upsert("Marker", { id: c.id, position: c.position, name: c.name ?? `marker ${tx.all("Marker").length + 1}`, color: c.color });
      break;
    case "Move":
      if (!(c.position >= 0)) fail("InvalidArgument", "marker position must be >= 0");
      tx.upsert("Marker", { ...marker(c.id), position: c.position });
      break;
    case "Rename":
      tx.upsert("Marker", { ...marker(c.id), name: c.name });
      break;
    case "SetColor":
      tx.upsert("Marker", { ...marker(c.id), color: c.color });
      break;
    case "Remove":
      for (const id of c.ids) {
        marker(id);
        tx.remove("Marker", id);
      }
      break;
  }
}

function audioClip(ctx: ReducerContext, id: string) {
  const cl = ctx.tx.get("Clip", id) ?? fail("NotFound", `clip ${id}`);
  if (cl.content.type !== "Audio") fail("InvalidArgument", `clip ${id} is not an audio clip`);
  return { clip: cl, content: cl.content };
}

export type ClipV2Command = Extract<ClipCommand, { type: "SetFadeCurves" | "SetReversed" | "Crossfade" | "SetCrossfade" }>;

export function clipV2Command(ctx: ReducerContext, c: ClipV2Command): void {
  const { tx } = ctx;
  switch (c.type) {
    case "SetFadeCurves": {
      checkCurve(c.fade_in);
      checkCurve(c.fade_out);
      const { clip, content } = audioClip(ctx, c.id);
      tx.upsert("Clip", {
        ...clip,
        content: { ...content, fade_in_curve: c.fade_in ?? content.fade_in_curve, fade_out_curve: c.fade_out ?? content.fade_out_curve },
      });
      break;
    }
    case "SetReversed": {
      const { clip, content } = audioClip(ctx, c.id);
      if (content.reversed === c.reversed) break;
      tx.upsert("Clip", { ...clip, ...mirrorReversed(ctx, clip), content: { ...content, reversed: c.reversed } });
      break;
    }
    case "Crossfade":
      checkCurve(c.curve);
      if (!(c.length > 0)) fail("InvalidArgument", "crossfade length must be > 0");
      crossfade(ctx, c.first, c.second, null, c.length, c.curve);
      break;
    case "SetCrossfade":
      checkCurve(c.curve);
      if (!Number.isFinite(c.at)) fail("InvalidArgument", "crossfade position must be finite");
      if (!(c.length >= 0)) fail("InvalidArgument", "crossfade length must be >= 0");
      crossfade(ctx, c.first, c.second, c.at, c.length, c.curve);
      break;
  }
}

/** `Beats::EPSILON`, like the controller's `clip_editing`. */
const EPS = BEATS_EPSILON;

/**
 * Mirror of Rust `clip_editing::is_crossfade`: the earlier clip ends inside the later one
 * and the overlap is covered by both fades (`ether_model::clip` overlap rules).
 */
export function isCrossfade(x: Clip, y: Clip): boolean {
  const [a, b] = x.start <= y.start ? [x, y] : [y, x];
  if (a.content.type !== "Audio" || b.content.type !== "Audio") return false;
  const aEnd = a.start + a.length;
  const overlap = aEnd - b.start;
  return overlap > 0 && aEnd < b.start + b.length + EPS && a.start < b.start - EPS && overlap <= a.content.fade_out + EPS && overlap <= b.content.fade_in + EPS;
}

/** Content beats of media (unwarped at the tempo of the clip start, repitched by transpose). */
function mediaBeats(ctx: ReducerContext, clip: Clip): number {
  if (clip.content.type !== "Audio") return 0;
  const m = ctx.tx.get("Media", clip.content.media);
  if (!m) return 0;
  const seconds = m.frames / Math.max(1, m.sample_rate);
  return (seconds * bpmAt(ctx.tx.project, clip.start)) / 60 / Math.pow(2, clip.content.transpose / 12);
}

/**
 * Mirror of Rust `ether_model::reverse::mirror` for an unwarped clip (the mock ignores warp
 * markers): offset and loop region mirrored over the file, so the reversed clip plays the
 * same material backwards (#442).
 */
function mirrorReversed(ctx: ReducerContext, clip: Clip): Pick<Clip, "offset" | "looping"> {
  if (clip.content.type !== "Audio") return clip;
  const m = ctx.tx.get("Media", clip.content.media);
  if (!m || !m.frames || !m.sample_rate) return clip;
  const k = ((m.frames / m.sample_rate) * bpmAt(ctx.tx.project, clip.start)) / 60;
  const rate = Math.pow(2, clip.content.transpose / 12);
  const axis = k + (1 - rate) * clip.length;
  const lp = clip.looping;
  const looping = { ...lp, start: Math.max(0, axis - lp.end), end: axis - lp.start };
  const looped = lp.enabled && lp.end - lp.start >= 1 / 256 && clip.offset < lp.end;
  const o = axis - clip.offset;
  const offset = looped ? (o >= looping.end - EPS ? looping.start : o) : o - clip.length;
  return { offset, looping };
}

const tailRoom = (ctx: ReducerContext, c: Clip, want: number) =>
  c.looping.enabled ? want : Math.max(0, Math.min(want, mediaBeats(ctx, c) - (c.offset + c.length)));
const headRoom = (c: Clip, want: number) => Math.max(0, Math.min(want, c.offset));

/**
 * Mirror of Rust `clip_editing::crossfade` (warp markers are ignored by the mock): centred
 * on `at` (null = the middle of the current overlap); `length` 0 removes the crossfade;
 * `curve` null keeps the clips' curves.
 */
function crossfade(ctx: ReducerContext, firstId: string, secondId: string, at: number | null, length: number, curve: FadeCurve | null): void {
  if (firstId === secondId) fail("InvalidArgument", "cannot crossfade a clip with itself");
  const a = audioClip(ctx, firstId);
  const b = audioClip(ctx, secondId);
  if (a.clip.track !== b.clip.track || (a.clip.lane ?? null) !== (b.clip.lane ?? null)) {
    fail("InvalidArgument", "crossfaded clips must be on the same track");
  }
  const [aStart, bStart] = [a.clip.start, b.clip.start];
  const [aEnd, bEnd] = [aStart + a.clip.length, bStart + b.clip.length];
  if (!(aStart < bStart - EPS && aEnd >= bStart - EPS && aEnd < bEnd + EPS)) {
    fail("InvalidArgument", "the first clip must start before the second and end where it starts or inside it");
  }
  let centre = (bStart + Math.min(aEnd, bEnd)) / 2;
  if (at !== null) {
    if (!(at > aStart + EPS && at < bEnd - EPS)) fail("InvalidArgument", "the crossfade must be centred inside both clips");
    centre = at;
  }
  const curveA = curve ?? a.content.fade_out_curve;
  const curveB = curve ?? b.content.fade_in_curve;
  if (length <= EPS) {
    removeCrossfade(ctx, a.clip, b.clip, Math.min(Math.max(centre, bStart), aEnd));
    return;
  }
  const half = length / 2;
  const wantA = Math.max(0, Math.min(centre + half - aEnd, bEnd - aEnd - EPS));
  const wantB = Math.max(0, Math.min(bStart - (centre - half), bStart - aStart - EPS));
  let extA = tailRoom(ctx, a.clip, wantA);
  let extB = headRoom(b.clip, wantB);
  const missing = wantA - extA + (wantB - extB);
  if (missing > EPS) {
    const moreA = Math.max(0, tailRoom(ctx, a.clip, Math.min(extA + missing, bEnd - aEnd - EPS)) - extA);
    extA += moreA;
    const rest = missing - moreA;
    if (rest > EPS) extB += Math.max(0, headRoom(b.clip, Math.min(extB + rest, bStart - aStart - EPS)) - extB);
  }
  const newAEnd = aEnd + extA - Math.max(0, aEnd - (centre + half));
  const newBStart = bStart - extB + Math.max(0, centre - half - bStart);
  const overlap = newAEnd - newBStart;
  if (overlap <= EPS) fail("InvalidState", "there is no source material left to crossfade these clips");
  const aLen = newAEnd - aStart;
  const bLen = bEnd - newBStart;
  const delta = bStart - newBStart;
  const first: Clip = {
    ...a.clip,
    length: aLen,
    content: { ...a.content, fade_out: overlap, fade_out_curve: curveA, fade_in: Math.min(a.content.fade_in, Math.max(0, aLen - overlap)) },
  };
  const second: Clip = {
    ...b.clip,
    start: newBStart,
    length: bLen,
    offset: Math.max(0, b.clip.offset - delta),
    content: { ...b.content, fade_in: overlap, fade_in_curve: curveB, fade_out: Math.min(b.content.fade_out, Math.max(0, bLen - overlap)) },
  };
  ctx.tx.upsert("Clip", first);
  ctx.tx.upsert("Clip", second);
}

/** Mirror of Rust `clip_editing::remove_crossfade`: the clips meet at `at`, crossfade fades reset. */
function removeCrossfade(ctx: ReducerContext, a: Clip, b: Clip, at: number): void {
  if (a.content.type !== "Audio" || b.content.type !== "Audio") return;
  const aLen = at - a.start;
  const delta = at - b.start;
  const bLen = b.length - delta;
  ctx.tx.upsert("Clip", { ...a, length: aLen, content: { ...a.content, fade_out: 0, fade_in: Math.min(a.content.fade_in, aLen) } });
  ctx.tx.upsert("Clip", {
    ...b,
    start: at,
    length: bLen,
    offset: b.offset + delta,
    content: { ...b.content, fade_in: 0, fade_out: Math.min(b.content.fade_out, bLen) },
  });
}
