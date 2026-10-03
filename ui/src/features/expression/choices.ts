/** Lane picker choices of the expression lane area. */

import type { ExpressionKind, NoteExpressionKind } from "@/generated";
import { kindKey } from "./model";

/** What the lane area shows. */
export type LaneChoice =
  | { type: "Velocity" }
  | { type: "Lane"; kind: ExpressionKind }
  | { type: "Note"; kind: NoteExpressionKind };

/** Controllers always offered by the picker (Ableton's defaults plus sustain). */
export const COMMON_KINDS: ReadonlyArray<ExpressionKind> = [
  { type: "PitchBend" },
  { type: "ChannelPressure" },
  { type: "Cc", controller: 1 },
  { type: "Cc", controller: 11 },
  { type: "Cc", controller: 64 },
];

/** Per-note kinds offered (the `mpe` node appends `Pitch` and `Timbre`). */
export const NOTE_KINDS: ReadonlyArray<NoteExpressionKind> = ["Pressure"];

export const OTHER_CC = "other-cc";

export function choiceKey(c: LaneChoice): string {
  return c.type === "Velocity" ? "velocity" : c.type === "Lane" ? kindKey(c.kind) : `note:${c.kind}`;
}
