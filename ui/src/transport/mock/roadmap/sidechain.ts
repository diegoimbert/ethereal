/**
 * Mock of `Device::SetSidechain`. Owned by `sidechain`. Mirrors
 * `crates/ether-controller/src/sidechain/mod.rs` + the model's rules (`ether_model::
 * SidechainTap`, routing cycles) through the selector's own checks
 * (`features/sidechain/routing.ts`): the source must be an existing non-master track, the
 * device must have a sidechain input (built-in descriptors; mock plugins have none), the tap
 * (a point of the track or of one of its rack chains / drum pads) must be heard before the
 * device runs and must not close a routing cycle. Devices at any depth (drum pads, rack
 * chains) take one. Clearing is always allowed; setting the current value is a no-op.
 */

import type { Device, SidechainTap, TrackId } from "@/generated";
import { POST_FADER, tapError, tapKey } from "@/features/sidechain/routing";
import { builtinDescriptor } from "../builtinDevices";
import { fail, type ReducerContext } from "../documentReducer";

/** Sidechain input channels of a device, as the mock engine reports them. */
export function sidechainInputs(d: Device): number {
  return d.kind.type === "Builtin" ? builtinDescriptor(d.kind.device).sidechain_inputs : 0;
}

export function setSidechain(ctx: ReducerContext, deviceId: string, source: TrackId | null, tap: SidechainTap | null = null): void {
  const d = ctx.tx.get("Device", deviceId) ?? fail("NotFound", `device ${deviceId}`);
  // The post-fader tap is stored as "no tap" (the original meaning).
  const next = source === null || tap === null || tapKey(tap) === tapKey(POST_FADER) ? null : tap;
  const old = d.sidechain_tap ?? null;
  if (d.sidechain === source && (old === null ? null : tapKey(old)) === (next === null ? null : tapKey(next))) return;
  if (source !== null) {
    const src = ctx.tx.get("Track", source) ?? fail("NotFound", `track ${source}`);
    if (src.kind === "Master") fail("InvalidArgument", "the master track cannot be a sidechain source");
    if (sidechainInputs(d) === 0) fail("InvalidArgument", `device ${d.id} has no sidechain input`);
    const p = {
      tracks: Object.fromEntries(ctx.tx.all("Track").map((t) => [t.id, t])),
      devices: Object.fromEntries(ctx.tx.all("Device").map((x) => [x.id, x])),
      sends: Object.fromEntries(ctx.tx.all("Send").map((x) => [x.id, x])),
      rack_chains: Object.fromEntries(ctx.tx.all("RackChain").map((x) => [x.id, x])),
      drum_pads: Object.fromEntries(ctx.tx.all("DrumPad").map((x) => [x.id, x])),
    };
    const why = tapError(p, d, source, next ?? POST_FADER);
    if (why === "loop") fail("InvalidArgument", "routing cycle");
    if (why !== null) fail("InvalidArgument", `sidechain: ${why}`);
  }
  const out: Device = { ...d, sidechain: source };
  if (next === null) delete out.sidechain_tap;
  else out.sidechain_tap = next;
  ctx.tx.upsert("Device", out);
}
