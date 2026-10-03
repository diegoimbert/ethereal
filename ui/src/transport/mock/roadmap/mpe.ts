/**
 * Mock of `Expression::SetTrackMpe` (v0.3, `mpe`): a MIDI track's MPE settings
 * (`Track::mpe`, one undo step; `null` turns MPE off), validated like the Rust controller
 * through the shared model (`@/features/mpe/model`); setting the current value is a no-op.
 *
 * The mock's MPE input: [`mpeTakeCommands`] gives the notes a simulated MPE controller
 * played on an MPE track (a slide into the note, a pressure swell and a timbre sweep), as
 * `SetNoteExpression` commands for the recording commit.
 */

import type { Command, ExpressionPoint, MpeSettings, NoteId, Project, TrackId } from "@/generated";
import { checkMpe, sameMpe } from "@/features/mpe/model";
import { cmd } from "../../cmd";
import { fail, type ReducerContext } from "../documentReducer";

export function setTrackMpe(ctx: ReducerContext, track: TrackId, mpe: MpeSettings | null): void {
  const t = ctx.tx.get("Track", track) ?? fail("NotFound", `track ${track}`);
  if (t.kind !== "Midi") fail("InvalidArgument", "MPE is for MIDI tracks");
  if (mpe) {
    const err = checkMpe(mpe);
    if (err) fail("InvalidArgument", err);
  }
  if (sameMpe(t.mpe, mpe)) return;
  const { mpe: _old, ...rest } = t;
  ctx.tx.upsert("Track", mpe ? { ...rest, mpe } : rest);
}

const step = (time: number, value: number): ExpressionPoint => ({ time, value, curve: { type: "Linear" } });

/**
 * What a simulated MPE controller played with recorded notes (`notes`: id, duration in
 * beats) on `track`: nothing unless the track has MPE. Each note slides up from a quarter
 * tone below over its first eighth, swells in pressure to its middle, and opens its timbre.
 */
export function mpeTakeCommands(
  project: Project,
  track: TrackId,
  notes: ReadonlyArray<{ id: NoteId; duration: number }>,
  newId: () => string,
): Command[] {
  if (!project.tracks[track]?.mpe) return [];
  const out: Command[] = [];
  for (const n of notes) {
    const d = Math.max(1 / 64, n.duration);
    const curves: Array<[ExpressionPoint[], "Pitch" | "Pressure" | "Timbre"]> = [
      [[step(0, -0.5), step(Math.min(0.125, d / 2), 0)], "Pitch"],
      [[step(0, 0.2), step(d / 2, 0.9), step(d, 0.5)], "Pressure"],
      [[step(0, 0.3), step(d, 0.8)], "Timbre"],
    ];
    for (const [points, kind] of curves) {
      out.push(cmd("Expression", { type: "SetNoteExpression", id: newId(), note: n.id, kind, points }));
    }
  }
  return out;
}
