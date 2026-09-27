/**
 * Mock of `Marker::*` and the v2 clip commands (`SetFadeCurves`, `SetReversed`,
 * `Crossfade`). Owned by `clip-editing`.
 */

import type { Clip, ClipCommand, FadeCurve, MarkerCommand } from "@/generated";
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
      tx.upsert("Marker", { id: c.id, position: c.position, name: c.name ?? `Marker ${tx.all("Marker").length + 1}`, color: c.color });
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

export type ClipV2Command = Extract<ClipCommand, { type: "SetFadeCurves" | "SetReversed" | "Crossfade" }>;

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
      tx.upsert("Clip", { ...clip, content: { ...content, reversed: c.reversed } });
      break;
    }
    case "Crossfade":
      crossfade(ctx, c);
      break;
  }
}

const EPS = 1e-9;

/**
 * Mirror of Rust `clip_editing::is_crossfade`: the earlier clip ends inside the later one
 * and the overlap is covered by both fades (`ether_model::clip` overlap rules).
 */
export function isCrossfade(x: Clip, y: Clip): boolean {
  const [a, b] = x.start <= y.start ? [x, y] : [y, x];
  if (a.content.type !== "Audio" || b.content.type !== "Audio") return false;
  const aEnd = a.start + a.length;
  const overlap = aEnd - b.start;
  return (
    overlap > 0 &&
    aEnd < b.start + b.length + EPS &&
    a.start < b.start - EPS &&
    overlap <= a.content.fade_out + EPS &&
    overlap <= b.content.fade_in + EPS
  );
}

/** Content beats of media (unwarped at the tempo of the clip start, repitched by transpose). */
function mediaBeats(ctx: ReducerContext, clip: Clip): number {
  if (clip.content.type !== "Audio") return 0;
  const m = ctx.tx.get("Media", clip.content.media);
  if (!m) return 0;
  const seconds = m.frames / Math.max(1, m.sample_rate);
  return (seconds * bpmAt(ctx.tx.project, clip.start)) / 60 / Math.pow(2, clip.content.transpose / 12);
}

const tailRoom = (ctx: ReducerContext, c: Clip, want: number) =>
  c.looping.enabled ? want : Math.max(0, Math.min(want, mediaBeats(ctx, c) - (c.offset + c.length)));
const headRoom = (c: Clip, want: number) => Math.max(0, Math.min(want, c.offset));

/** Mirror of Rust `clip_editing::crossfade` (warp markers are ignored by the mock). */
function crossfade(ctx: ReducerContext, c: Extract<ClipCommand, { type: "Crossfade" }>): void {
  checkCurve(c.curve);
  if (!(c.length > 0)) fail("InvalidArgument", "crossfade length must be > 0");
  if (c.first === c.second) fail("InvalidArgument", "cannot crossfade a clip with itself");
  const a = audioClip(ctx, c.first);
  const b = audioClip(ctx, c.second);
  if (a.clip.track !== b.clip.track) fail("InvalidArgument", "crossfaded clips must be on the same track");
  const [aStart, bStart] = [a.clip.start, b.clip.start];
  const [aEnd, bEnd] = [aStart + a.clip.length, bStart + b.clip.length];
  if (!(aStart < bStart - EPS && aEnd >= bStart - EPS && aEnd < bEnd + EPS)) {
    fail("InvalidArgument", "the first clip must start before the second and end where it starts or inside it");
  }
  const centre = (bStart + Math.min(aEnd, bEnd)) / 2;
  const half = c.length / 2;
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
    content: { ...a.content, fade_out: overlap, fade_out_curve: c.curve, fade_in: Math.min(a.content.fade_in, Math.max(0, aLen - overlap)) },
  };
  const second: Clip = {
    ...b.clip,
    start: newBStart,
    length: bLen,
    offset: Math.max(0, b.clip.offset - delta),
    content: { ...b.content, fade_in: overlap, fade_in_curve: c.curve, fade_out: Math.min(b.content.fade_out, Math.max(0, bLen - overlap)) },
  };
  ctx.tx.upsert("Clip", first);
  ctx.tx.upsert("Clip", second);
}
