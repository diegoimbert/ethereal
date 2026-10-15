/** The commands behind a Solo click, from `planSolo` (one undo step each). */

import type { Command, RackChain, RackChainId, Track, TrackId } from "@/generated";
import { cmd } from "@/transport";
import { planSolo } from "./soloPlan";

/**
 * Arranger / mixer / inspector track solo. The scope is every track but the master. The
 * exclusive form is a single `SetSolo { exclusive }` (the document unsolos the others itself, so
 * collab sites only touch what they see).
 */
export function trackSoloCommand(tracks: readonly Track[], id: TrackId, additive: boolean): Command {
  const scope = tracks.filter((t) => t.kind !== "Master").map((t) => ({ id: t.id, solo: t.mixer.solo }));
  const plan = planSolo(scope, id, additive);
  const solo = plan.exclusive ? true : (plan.changes[0]?.solo ?? true);
  return cmd("Mixer", { type: "SetSolo", track: id, solo, exclusive: plan.exclusive });
}

/** Rack chain solo: scope is the chains of the clicked chain's rack. One `Edit::Batch` when several change. */
export function chainSoloCommand(chains: readonly RackChain[], id: RackChainId, additive: boolean): Command {
  const clicked = chains.find((c) => c.id === id);
  const scope = chains.filter((c) => c.rack === clicked?.rack).map((c) => ({ id: c.id, solo: c.solo }));
  const commands = planSolo(scope, id, additive).changes.map((c) =>
    cmd("Rack", { type: "SetChainMix", id: c.id, volume: null, pan: null, mute: null, solo: c.solo }),
  );
  // The label is the English for agents and logs; the UI shows the key.
  return commands.length === 1 ? commands[0]! : cmd("Edit", { type: "Batch", label: "solo chain", key: { id: "undo.rack.solo_chain", args: {} }, commands }); // i18n-ignore
}
