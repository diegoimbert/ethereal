/**
 * Mock of `Expression::*` (v0.3, contracts-4). Owned by `midi-expression`: clip expression
 * lanes (CC / pitch bend / channel pressure) and per-note expressions as ordinary undoable
 * document edits (CONTRACTS.md §13.2), validated like the Rust controller through the shared
 * pure model (`@/features/expression/model`). `SetTrackMpe` is the `mpe` node's
 * (`./mpe.ts`).
 *
 * Copies and cascades (Rust `doc/mod.rs`, `doc/notes.rs`): [`copyClipExpression`],
 * [`copyNoteExpressions`], [`deleteClipExpression`] and [`deleteNoteExpressions`] are the
 * helpers the document reducer calls where clips/notes are deleted or copied.
 */

import type { ClipId, ExpressionCommand, ExpressionPoint, NoteExpressionKind, NoteId } from "@/generated";
import {
  checkPoints,
  expressionRange,
  MAX_EXPRESSION_CC,
  noteExpressionRange,
  replaceRange,
  sameKind,
  type Range,
} from "@/features/expression/model";
import { fail, type ReducerContext } from "../documentReducer";
import type { Tx } from "../tx";
import { setTrackMpe } from "./mpe";

function check(points: ReadonlyArray<ExpressionPoint>, range: Range): void {
  const err = checkPoints(points, range);
  if (err) fail("InvalidArgument", err);
}

export function expressionCommand(ctx: ReducerContext, c: ExpressionCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "SetTrackMpe":
      return setTrackMpe(ctx, c.track, c.mpe);
    case "CreateLane": {
      if (tx.get("ExpressionLane", c.id)) return;
      const clip = tx.get("Clip", c.clip);
      if (!clip) fail("NotFound", `clip ${c.clip}`);
      if (clip.content.type !== "Midi") fail("InvalidArgument", "expression lanes can only live in MIDI clips");
      if (c.kind.type === "Cc" && !(c.kind.controller >= 0 && c.kind.controller <= MAX_EXPRESSION_CC)) {
        fail("InvalidArgument", `expression lane CC must be 0..=${MAX_EXPRESSION_CC}`);
      }
      if (tx.all("ExpressionLane").some((l) => l.clip === c.clip && sameKind(l.kind, c.kind))) return;
      tx.upsert("ExpressionLane", { id: c.id, clip: c.clip, kind: c.kind, points: [] });
      return;
    }
    case "RemoveLane":
      if (!tx.get("ExpressionLane", c.id)) fail("NotFound", `expression lane ${c.id}`);
      tx.remove("ExpressionLane", c.id);
      return;
    case "SetPoints": {
      const lane = tx.get("ExpressionLane", c.lane);
      if (!lane) fail("NotFound", `expression lane ${c.lane}`);
      check(c.points, expressionRange(lane.kind));
      tx.upsert("ExpressionLane", { ...lane, points: c.points });
      return;
    }
    case "ReplaceRange": {
      const lane = tx.get("ExpressionLane", c.lane);
      if (!lane) fail("NotFound", `expression lane ${c.lane}`);
      const next = replaceRange(lane.points, c.start, c.end, c.points);
      if (typeof next === "string") fail("InvalidArgument", next);
      check(next, expressionRange(lane.kind));
      tx.upsert("ExpressionLane", { ...lane, points: next });
      return;
    }
    case "SetNoteExpression": {
      if (!tx.get("Note", c.note)) fail("NotFound", `note ${c.note}`);
      check(c.points, noteExpressionRange(c.kind));
      const existing = tx.all("NoteExpression").find((e) => e.note === c.note && e.kind === c.kind);
      if (existing) {
        if (c.points.length === 0) tx.remove("NoteExpression", existing.id);
        else tx.upsert("NoteExpression", { ...existing, points: c.points });
      } else if (c.points.length > 0 && !tx.get("NoteExpression", c.id)) {
        tx.upsert("NoteExpression", { id: c.id, note: c.note, kind: c.kind, points: c.points });
      }
      return;
    }
    case "ClearNoteExpressions":
      deleteNoteExpressions(tx, c.notes, c.kind);
      return;
  }
}

/** Remove the expressions of `kind` (all when `null`) of these notes. */
export function deleteNoteExpressions(tx: Tx, notes: ReadonlyArray<NoteId>, kind: NoteExpressionKind | null = null): void {
  const set = new Set(notes);
  for (const e of tx.all("NoteExpression")) {
    if (set.has(e.note) && (kind === null || e.kind === kind)) tx.remove("NoteExpression", e.id);
  }
}

/** Cascade: a deleted clip's lanes go with it. */
export function deleteClipExpression(tx: Tx, clip: ClipId): void {
  for (const l of tx.all("ExpressionLane")) if (l.clip === clip) tx.remove("ExpressionLane", l.id);
}

/** Copy `from`'s lanes onto clip `to` (fresh ids). */
export function copyClipExpression(ctx: ReducerContext, from: ClipId, to: ClipId): void {
  for (const l of ctx.tx.all("ExpressionLane")) {
    if (l.clip === from) ctx.tx.upsert("ExpressionLane", { ...l, id: ctx.newId(), clip: to });
  }
}

/** Copy note `from`'s expressions onto note `to` (fresh ids). */
export function copyNoteExpressions(ctx: ReducerContext, from: NoteId, to: NoteId): void {
  for (const e of ctx.tx.all("NoteExpression")) {
    if (e.note === from) ctx.tx.upsert("NoteExpression", { ...e, id: ctx.newId(), note: to });
  }
}
