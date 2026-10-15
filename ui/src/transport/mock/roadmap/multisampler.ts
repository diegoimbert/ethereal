/**
 * Mock of `Device::SetZones` (v0.2, `multisampler`): replace a multisampler's zones
 * (`BuiltinDevice::MultiSampler::zones`, one undo step), validating like the engine
 * (`crates/ether-controller/src/multisampler`, `ether_model::apply` zone checks): the device
 * must be a multisampler, at most `MAX_ZONES` zones, known media, ranges in order, root key
 * 0..=127, tune within ±100 cents, non-negative times, pan in -1..=1.
 */

import type { DeviceId, SampleZone, Zone } from "@/generated";
import { fail, type ReducerContext } from "../documentReducer";

/** `ether_model::multisampler::MAX_ZONES`. */
export const MAX_ZONES = 512;

function checkRange(what: string, z: Zone): void {
  if (!(Number.isInteger(z.lo) && Number.isInteger(z.hi) && z.lo >= 0 && z.hi <= 127 && z.lo <= z.hi)) {
    fail("InvalidArgument", `${what} zone must satisfy 0 <= lo <= hi <= 127`);
  }
}

function checkZone(z: SampleZone): void {
  if (!(Number.isInteger(z.root_key) && z.root_key >= 0 && z.root_key <= 127)) fail("InvalidArgument", "zone root key must be 0..=127");
  if (!(Number.isFinite(z.tune_cents) && Math.abs(z.tune_cents) <= 100)) fail("InvalidArgument", "zone tune must be within ±100 cents");
  checkRange("key", z.keys);
  checkRange("velocity", z.velocities);
  for (const [what, s] of [
    ["zone start", z.start],
    ["zone loop start", z.loop_start],
    ["zone loop end", z.loop_end],
    ["zone loop crossfade", z.loop_crossfade],
    ["zone end", z.end ?? 0],
  ] as const) {
    if (!(Number.isFinite(s) && s >= 0)) fail("InvalidArgument", `${what} must be >= 0`);
  }
  if (!Number.isFinite(z.gain)) fail("InvalidArgument", "zone gain must be finite");
  if (!(z.pan >= -1 && z.pan <= 1)) fail("InvalidArgument", "pan must be in -1..=1");
}

export function setZones(ctx: ReducerContext, device: DeviceId, zones: SampleZone[]): void {
  const { tx } = ctx;
  const d = tx.get("Device", device) ?? fail("NotFound", `device ${device}`);
  if (d.kind.type !== "Builtin" || d.kind.device.type !== "MultiSampler") {
    fail("InvalidArgument", `device ${d.id} is not a multisampler`);
  }
  if (zones.length > MAX_ZONES) fail("InvalidArgument", `at most ${MAX_ZONES} zones`);
  for (const z of zones) {
    if (z.media !== null && !tx.get("Media", z.media)) fail("NotFound", `media ${z.media}`);
    checkZone(z);
  }
  tx.upsert("Device", { ...d, kind: { type: "Builtin", device: { type: "MultiSampler", zones: zones.map((z) => ({ ...z })) } } });
}
