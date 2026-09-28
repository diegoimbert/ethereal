/**
 * Mock of `Take::*` (v0.2, `comping`): take lanes, swipe comping (`SetComp`), crossfades,
 * flatten, audition. Mirrors `crates/ether-controller/src/comping` (region math and comp
 * pieces are shared with the UI: `@/features/comping/model`). The mock plays no audio, so
 * `Audition` only validates (runtime, no document change, like the engine).
 */

import type { Clip, Note, TakeCommand, TakeLaneId, TrackId } from "@/generated";
import {
  clearRegions,
  compOf,
  compPieces,
  deriveId,
  laneClipsOf,
  lanesOf,
  MAX_COMP_CROSSFADE,
  nextTakeName,
  swipeRegions,
  type RegionEdit,
} from "@/features/comping/model";
import { keyForInsert } from "@/state/orderKey";
import { fail, type ReducerContext } from "../documentReducer";
import { bpmAt } from "../tempo";

function trackOf(ctx: ReducerContext, id: TrackId) {
  return ctx.tx.get("Track", id) ?? fail("NotFound", `track ${id}`);
}

function laneOf(ctx: ReducerContext, id: TakeLaneId) {
  return ctx.tx.get("TakeLane", id) ?? fail("NotFound", `take lane ${id}`);
}

function checkRange(start: number, end: number): void {
  if (!(Number.isFinite(start) && Number.isFinite(end) && start >= 0 && end > start + 1e-6)) {
    fail("InvalidArgument", "comp range must satisfy 0 <= start < end");
  }
}

function applyEdits(ctx: ReducerContext, edits: RegionEdit[]): void {
  for (const e of edits) {
    if (e.type === "remove") ctx.tx.remove("CompRegion", e.id);
    else ctx.tx.upsert("CompRegion", e.region);
  }
}

function deleteClip(ctx: ReducerContext, id: string): void {
  const { tx } = ctx;
  for (const n of tx.all("Note")) if (n.clip === id) tx.remove("Note", n.id);
  for (const m of tx.all("WarpMarker")) if (m.clip === id) tx.remove("WarpMarker", m.id);
  for (const l of tx.all("AutomationLane")) {
    if (l.owner.type !== "Clip" || l.owner.clip !== id) continue;
    for (const p of tx.all("AutomationPoint")) if (p.lane === l.id) tx.remove("AutomationPoint", p.id);
    tx.remove("AutomationLane", l.id);
  }
  tx.remove("Clip", id);
}

function removeLane(ctx: ReducerContext, id: TakeLaneId): void {
  for (const r of ctx.tx.all("CompRegion")) if (r.lane === id) ctx.tx.remove("CompRegion", r.id);
  for (const c of laneClipsOf(ctx.tx.project, id)) deleteClip(ctx, c.id);
  ctx.tx.remove("TakeLane", id);
}

/** Track deleted (core cascade, after its clips): its regions and lanes go. */
export function onTrackDeletedTakes(ctx: ReducerContext, track: TrackId): void {
  for (const r of ctx.tx.all("CompRegion")) if (r.track === track) ctx.tx.remove("CompRegion", r.id);
  for (const l of ctx.tx.all("TakeLane")) if (l.track === track) ctx.tx.remove("TakeLane", l.id);
}

function flatten(ctx: ReducerContext, c: Extract<TakeCommand, { type: "Flatten" }>): void {
  const t = trackOf(ctx, c.track);
  if (t.kind !== "Audio" && t.kind !== "Midi") fail("InvalidArgument", "only audio and MIDI tracks have takes");
  const project = ctx.tx.project;
  const pieces = compPieces(project, c.track, (b) => bpmAt(project, b));
  let noteIndex = 0;
  pieces.forEach((piece, i) => {
    const src = ctx.tx.get("Clip", piece.clip)!;
    const id = deriveId(c.seed, i);
    if (ctx.tx.get("Clip", id)) fail("InvalidArgument", `clip ${id} already exists`);
    const length = piece.end - piece.start;
    const { lane: _lane, ...rest } = src;
    void _lane;
    const clip: Clip = { ...rest, id, start: piece.start, length, offset: piece.offset };
    if (clip.content.type === "Audio") {
      const a = { ...clip.content };
      if (piece.fadeIn !== null) Object.assign(a, { fade_in: piece.fadeIn, fade_in_curve: { type: "EqualPower" } });
      if (piece.fadeOut !== null) Object.assign(a, { fade_out: piece.fadeOut, fade_out_curve: { type: "EqualPower" } });
      a.fade_in = Math.min(a.fade_in, length);
      a.fade_out = Math.min(a.fade_out, length - a.fade_in);
      clip.content = a;
    }
    ctx.tx.upsert("Clip", clip);
    const c0 = piece.offset;
    const c1 = piece.offset + length;
    const notes = ctx.tx
      .all("Note")
      .filter((n) => n.clip === src.id && (src.looping.enabled || (n.start >= c0 - 1e-6 && n.start < c1 - 1e-6)))
      .sort((a, b) => a.start - b.start || a.pitch - b.pitch || (a.id < b.id ? -1 : 1));
    for (const n of notes) ctx.tx.upsert("Note", { ...n, id: deriveId(c.seed_notes, noteIndex++), clip: id } satisfies Note);
    ctx.tx
      .all("WarpMarker")
      .filter((m) => m.clip === src.id)
      .forEach((m, k) => ctx.tx.upsert("WarpMarker", { ...m, id: deriveId(id, k), clip: id }));
  });
  for (const r of compOf(project, c.track)) ctx.tx.remove("CompRegion", r.id);
  if (!c.keep_lanes) for (const l of lanesOf(project, c.track)) removeLane(ctx, l.id);
}

/** Document command (one undo step), dispatched from `roadmap/index.ts`. */
export function takeCommand(ctx: ReducerContext, c: TakeCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "CreateLane": {
      if (tx.get("TakeLane", c.id)) return;
      const t = trackOf(ctx, c.track);
      if (t.kind !== "Audio" && t.kind !== "Midi") fail("InvalidArgument", "take lanes belong to audio or MIDI tracks");
      const siblings = lanesOf(tx.project, c.track);
      if (c.before !== null && !siblings.some((l) => l.id === c.before)) fail("InvalidArgument", `${c.before} is not a sibling`);
      const name = c.name?.trim() ? c.name.trim() : nextTakeName(siblings);
      tx.upsert("TakeLane", { id: c.id, track: c.track, order: keyForInsert(siblings, c.before), name, color: null });
      return;
    }
    case "RemoveLane":
      laneOf(ctx, c.id);
      removeLane(ctx, c.id);
      return;
    case "RenameLane": {
      const l = laneOf(ctx, c.id);
      if (!c.name.trim()) fail("InvalidArgument", "a take lane needs a name");
      tx.upsert("TakeLane", { ...l, name: c.name.trim() });
      return;
    }
    case "SetLaneColor":
      tx.upsert("TakeLane", { ...laneOf(ctx, c.id), color: c.color });
      return;
    case "MoveLane": {
      const l = laneOf(ctx, c.id);
      if (c.before === c.id) return;
      const siblings = lanesOf(tx.project, l.track).filter((s) => s.id !== c.id);
      if (c.before !== null && !siblings.some((s) => s.id === c.before)) fail("InvalidArgument", `${c.before} is not a sibling`);
      tx.upsert("TakeLane", { ...l, order: keyForInsert(siblings, c.before) });
      return;
    }
    case "MoveToLane": {
      const to = c.lane === null ? null : laneOf(ctx, c.lane);
      for (const id of c.clips) {
        const clip = tx.get("Clip", id) ?? fail("NotFound", `clip ${id}`);
        if (to && to.track !== clip.track) fail("InvalidState", "a take clip must be on its lane's track");
        const { lane: _lane, ...rest } = clip;
        void _lane;
        tx.upsert("Clip", to ? { ...rest, lane: to.id } : rest);
      }
      return;
    }
    case "SetComp": {
      if (tx.get("CompRegion", c.id)) return;
      checkRange(c.start, c.end);
      const l = laneOf(ctx, c.lane);
      if (l.track !== c.track) fail("InvalidArgument", `take lane ${c.lane} is not on track ${c.track}`);
      applyEdits(ctx, swipeRegions(compOf(tx.project, c.track), { id: c.id, splitId: c.split_id, track: c.track, lane: c.lane, start: c.start, end: c.end }));
      return;
    }
    case "ClearComp":
      trackOf(ctx, c.track);
      checkRange(c.start, c.end);
      applyEdits(ctx, clearRegions(compOf(tx.project, c.track), c.start, c.end, c.split_id));
      return;
    case "SetCrossfade": {
      const r = tx.get("CompRegion", c.region) ?? fail("NotFound", `comp region ${c.region}`);
      if (!(Number.isFinite(c.crossfade) && c.crossfade >= 0 && c.crossfade <= MAX_COMP_CROSSFADE)) {
        fail("InvalidArgument", `comp crossfade must be 0..=${MAX_COMP_CROSSFADE} s`);
      }
      tx.upsert("CompRegion", { ...r, crossfade: c.crossfade });
      return;
    }
    case "Flatten":
      flatten(ctx, c);
      return;
    case "Audition":
      trackOf(ctx, c.track);
      if (c.lane !== null && laneOf(ctx, c.lane).track !== c.track) fail("InvalidArgument", `take lane ${c.lane} is not on track ${c.track}`);
      return;
  }
}
