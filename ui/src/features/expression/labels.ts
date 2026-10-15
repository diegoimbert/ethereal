/**
 * Lane names as the UI shows them, in the user's language ("pitch bend", "cc 1 mod wheel",
 * "note pitch"). `@/domain/expression`'s `kindLabel` stays the stable English name (undo
 * labels, `data-kind`).
 */

import type { ExpressionKind, NoteExpressionKind, TextKey } from "@/generated";
import { tText, type MessageKey } from "@/i18n";

const CC_NAMES: Readonly<Record<number, MessageKey>> = {
  1: "expression.cc.mod_wheel",
  2: "expression.cc.breath",
  7: "expression.cc.volume",
  10: "expression.cc.pan",
  11: "expression.cc.expression",
  64: "expression.cc.sustain",
  74: "expression.cc.brightness",
};

const key = (id: MessageKey, args: TextKey["args"] = {}): TextKey => ({ id, args });

/** A clip expression lane's name as a message (also an argument of its undo labels). */
export function kindKey(kind: ExpressionKind): TextKey {
  switch (kind.type) {
    case "PitchBend":
      return key("expression.kind.pitch_bend");
    case "ChannelPressure":
      return key("expression.kind.channel_pressure");
    case "Cc": {
      const name = CC_NAMES[kind.controller];
      return name ? key("expression.kind.cc_named", { n: kind.controller, name: key(name) }) : key("expression.kind.cc", { n: kind.controller });
    }
  }
}

/** A per-note lane's name as a message. */
export function noteKindKey(kind: NoteExpressionKind): TextKey {
  return key(kind === "Pitch" ? "expression.note.pitch" : kind === "Pressure" ? "expression.note.pressure" : "expression.note.timbre");
}

/** A clip expression lane's name. */
export function kindText(kind: ExpressionKind): string {
  return tText(kindKey(kind), "");
}

/** A per-note lane's name. */
export function noteKindText(kind: NoteExpressionKind): string {
  return tText(noteKindKey(kind), "");
}
