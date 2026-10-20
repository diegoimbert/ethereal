/**
 * Mock of `ClipCommand::SetTranspose`, mirroring
 * `ether-controller` `clip_editing::transpose`: when the transpose changes the playback
 * speed (unwarped or Repitch), the clip keeps its audio and its length and loop region
 * follow (positions past the pivot `max(offset, 0)` move to where they read the same source
 * second); a longer clip is trimmed at the next clip of its lane; fades are clamped.
 */

import type { Clip } from "@/generated";
import { BEATS_EPSILON } from "@/state/beats";
import { beatAtSource, compileWarp, sourceSecondsAt } from "@/domain/warpMap";
import { fail, type ReducerContext } from "../documentReducer";
import { bpmAt } from "../tempo";

const EPS = BEATS_EPSILON;

export function setTranspose(ctx: ReducerContext, cl: Clip, semitones: number): Clip {
  if (cl.content.type !== "Audio") fail("InvalidArgument", `clip ${cl.id} is not an audio clip`);
  if (!Number.isFinite(semitones)) fail("InvalidArgument", "transpose must be finite");
  const a = cl.content;
  const t = Math.min(48, Math.max(-48, semitones));
  if (t === a.transpose) return cl;
  const out: Clip = { ...cl, content: { ...a, transpose: t } };
  const markers = ctx.tx.all("WarpMarker").filter((m) => m.clip === cl.id);
  const pins = compileWarp(a.warp, markers);
  if (pins && a.warp.mode !== "Repitch") return out; // pitch-preserving: same duration
  const refBpm = bpmAt(ctx.tx.project, cl.start);
  const map = (c: number) => sourceSecondsAt(pins, refBpm, c);
  const k = Math.pow(2, (a.transpose - t) / 12);
  const pivot = Math.max(cl.offset, 0);
  const sp = map(pivot);
  const remap = (c: number) => {
    if (c < 0) return c;
    return pins ? beatAtSource(map, sp + (map(c) - sp) * k) : pivot + (c - pivot) * k;
  };

  let len = remap(cl.offset + cl.length) - cl.offset;
  if (len > cl.length) {
    const next = ctx.tx
      .all("Clip")
      .filter((o) => o.id !== cl.id && o.track === cl.track && (o.lane ?? null) === (cl.lane ?? null) && o.start > cl.start + EPS)
      .reduce((m, o) => Math.min(m, o.start), Infinity);
    len = Math.min(len, Math.max(next - cl.start, cl.length));
  }
  len = Math.max(len, EPS * 10);
  const ls = Math.max(remap(cl.looping.start), 0);
  const le = remap(cl.looping.end);
  return {
    ...out,
    length: len,
    looping: le > ls ? { ...cl.looping, start: ls, end: le } : cl.looping,
    content: { ...a, transpose: t, fade_in: Math.min(a.fade_in, len), fade_out: Math.min(a.fade_out, len) },
  };
}
