/**
 * Mock of `Marker::*` and the v2 clip commands (`SetFadeCurves`, `SetReversed`,
 * `Crossfade`). Owned by `clip-editing`.
 */

import type { Clip, ClipCommand, FadeCurve, MarkerCommand } from "@/generated";
import { fail, type ReducerContext } from "../documentReducer";

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
    case "Crossfade": {
      checkCurve(c.curve);
      if (!(c.length > 0)) fail("InvalidArgument", "crossfade length must be > 0");
      const a = audioClip(ctx, c.first);
      const b = audioClip(ctx, c.second);
      if (a.clip.track !== b.clip.track) fail("InvalidArgument", "crossfaded clips must be on the same track");
      // The mock simply extends the first clip so the overlap is `length` (no source checks).
      const end = Math.max(a.clip.start + a.clip.length, b.clip.start + c.length);
      const first: Clip = {
        ...a.clip,
        length: end - a.clip.start,
        content: { ...a.content, fade_out: c.length, fade_out_curve: c.curve },
      };
      tx.upsert("Clip", first);
      tx.upsert("Clip", { ...b.clip, content: { ...b.content, fade_in: c.length, fade_in_curve: c.curve } });
      break;
    }
  }
}
