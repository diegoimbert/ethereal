/** Lane picker choices of the expression lane area. */

import type { ExpressionKind, NoteExpressionKind } from "@/generated";
import { ENVELOPE_PREFIX } from "@/features/automation";
import { kindKey } from "@/domain/expression";

/** What the lane area shows. */
export type LaneChoice =
  | { type: "Velocity" }
  | { type: "Lane"; kind: ExpressionKind }
  | { type: "Note"; kind: NoteExpressionKind }
  /** A clip envelope (automation owned by the clip), by its target key. */
  | { type: "Envelope"; key: string };

/** Controllers always offered by the picker (Ableton's defaults plus sustain). */
export const COMMON_KINDS: ReadonlyArray<ExpressionKind> = [
  { type: "PitchBend" },
  { type: "ChannelPressure" },
  { type: "Cc", controller: 1 },
  { type: "Cc", controller: 11 },
  { type: "Cc", controller: 64 },
];

/** Per-note kinds offered (`Pitch` and `Timbre` from the `mpe` node). */
export const NOTE_KINDS: ReadonlyArray<NoteExpressionKind> = ["Pressure", "Pitch", "Timbre"];

export const OTHER_CC = "other-cc";

export function choiceKey(c: LaneChoice): string {
  switch (c.type) {
    case "Velocity":
      return "velocity";
    case "Lane":
      return kindKey(c.kind);
    case "Note":
      return `note:${c.kind}`;
    case "Envelope":
      return ENVELOPE_PREFIX + c.key;
  }
}
