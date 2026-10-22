/**
 * The arrangement's clip badge for a MIDI clip's own controller data (its expression lanes
 * with points): "sustain", "bend · sustain", "3 controllers". Edited in the automation tab
 * (`ClipControllerLanes`) and the piano roll's lane area.
 */

import type { ExpressionKind } from "@/generated";
import { t } from "@/i18n";
import { kindKey } from "@/domain/expression";
import { kindText } from "./labels";

const CC_NAMES: Readonly<Record<number, "expression.cc.mod_wheel" | "expression.cc.expression" | "expression.cc.sustain">> = {
  1: "expression.cc.mod_wheel",
  11: "expression.cc.expression",
  64: "expression.cc.sustain",
};

/** A controller's short name ("sustain", "bend", "cc 20"). */
function shortName(kind: ExpressionKind): string {
  switch (kind.type) {
    case "PitchBend":
      return t("expression.badge.bend");
    case "ChannelPressure":
      return t("expression.badge.pressure");
    case "Cc": {
      const name = CC_NAMES[kind.controller];
      return name ? t(name) : kindText(kind);
    }
  }
}

/** Controller order: pitch bend, channel pressure, then CCs by number. */
export function compareKinds(a: ExpressionKind, b: ExpressionKind): number {
  const rank = (k: ExpressionKind) => (k.type === "PitchBend" ? -2 : k.type === "ChannelPressure" ? -1 : k.controller);
  return rank(a) - rank(b);
}

/** The badge text for the kinds of a clip's non-empty lanes, or null without any. */
export function controllerBadgeText(kinds: ReadonlyArray<ExpressionKind>): string | null {
  const unique = [...new Map(kinds.map((k) => [kindKey(k), k])).values()];
  if (unique.length === 0) return null;
  if (unique.length > 2) return t("expression.badge.many", { count: unique.length });
  return unique.sort(compareKinds).map(shortName).join(" · ");
}
