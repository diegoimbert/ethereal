/**
 * Mock of `Device::SetSidechain`. Owned by `sidechain`. Mirrors
 * `crates/ether-controller/src/sidechain/mod.rs` + the model's routing check: the source must
 * be an existing non-master track other than the device's own, the device must have a
 * sidechain input (built-in descriptors; mock plugins have none) and must not be in a
 * drum-rack pad chain, and the sidechain edge `source → device track` must not close a
 * routing cycle (outputs + sends + sidechains). Clearing is always allowed; setting the
 * current value is a no-op.
 */

import type { Device, TrackId } from "@/generated";
import { builtinDescriptor } from "../builtinDevices";
import { fail, type ReducerContext } from "../documentReducer";

/** Sidechain input channels of a device, as the mock engine reports them. */
export function sidechainInputs(d: Device): number {
  return d.kind.type === "Builtin" ? builtinDescriptor(d.kind.device).sidechain_inputs : 0;
}

/** `true` if `to` is reachable from `from` in the routing graph (outputs, sends, sidechains). */
function reaches(ctx: ReducerContext, from: TrackId, to: TrackId): boolean {
  const tracks = ctx.tx.all("Track");
  const master = tracks.find((t) => t.kind === "Master")?.id ?? null;
  const edges = new Map<TrackId, TrackId[]>();
  const add = (a: TrackId, b: TrackId) => edges.set(a, [...(edges.get(a) ?? []), b]);
  for (const t of tracks) {
    if (t.output.type === "Track") add(t.id, t.output.track);
    else if (t.output.type === "Default" && t.kind !== "Master") {
      const dest = t.parent ?? master;
      if (dest !== null) add(t.id, dest);
    }
  }
  for (const s of ctx.tx.all("Send")) add(s.from, s.to);
  for (const d of ctx.tx.all("Device")) if (d.sidechain !== null) add(d.sidechain, d.track);
  const seen = new Set<TrackId>();
  const stack = [from];
  while (stack.length > 0) {
    const t = stack.pop()!;
    if (t === to) return true;
    if (seen.has(t)) continue;
    seen.add(t);
    stack.push(...(edges.get(t) ?? []));
  }
  return false;
}

export function setSidechain(ctx: ReducerContext, deviceId: string, source: TrackId | null): void {
  const d = ctx.tx.get("Device", deviceId) ?? fail("NotFound", `device ${deviceId}`);
  if (d.sidechain === source) return;
  if (source !== null) {
    const src = ctx.tx.get("Track", source) ?? fail("NotFound", `track ${source}`);
    if (src.kind === "Master") fail("InvalidArgument", "the master track cannot be a sidechain source");
    if (d.pad !== null) fail("InvalidArgument", `device ${d.id} is in a drum-rack pad chain and cannot take a sidechain`);
    if (sidechainInputs(d) === 0) fail("InvalidArgument", `device ${d.id} has no sidechain input`);
    if (source === d.track) fail("InvalidArgument", "a device cannot sidechain its own track");
    // The new edge source → d.track closes a cycle iff source is reachable from d.track.
    if (reaches(ctx, d.track, source)) fail("InvalidArgument", "routing cycle");
  }
  ctx.tx.upsert("Device", { ...d, sidechain: source });
}
