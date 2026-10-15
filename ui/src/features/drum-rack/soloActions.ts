/**
 * A click on a pad's Solo (pad grid menu, pad settings). Scope: the pads of that pad's drum
 * rack. Pad solo is runtime engine state (`DrumRack::SetPadSolo`, no document change, no undo
 * step), so there is no batch: the target is soloed first, then the others unsoloed, so the rack
 * never goes silent in between.
 */

import type { DrumPad } from "@/generated";
import { planSolo } from "@/domain/soloPlan";
import { useProjectStore } from "@/state";
import { cmd, type useTransport } from "@/transport";
import { reportFailure } from "@/features/devices/gesture";
import { useDrumSolo } from "./soloStore";

export function soloPad(transport: Pick<ReturnType<typeof useTransport>, "send">, pad: DrumPad, additive: boolean): void {
  const store = useDrumSolo.getState();
  const pads = Object.values(useProjectStore.getState().project?.drum_pads ?? {}).filter((p) => p.rack === pad.rack);
  const scope = pads.map((p) => ({ id: p.id, solo: store.soloed.has(p.id) }));
  for (const c of planSolo(scope, pad.id, additive).changes) {
    store.set(c.id, c.solo);
    void reportFailure(transport.send(cmd("DrumRack", { type: "SetPadSolo", id: c.id, solo: c.solo })));
  }
}
