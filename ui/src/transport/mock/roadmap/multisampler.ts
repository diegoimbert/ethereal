/**
 * Mock of `Device::SetZones` (v0.2, contracts-3). Owned by `multisampler`: replace the
 * multisampler's zones (`BuiltinDevice::MultiSampler::zones`, one undo step), rejecting
 * missing media like the engine. Until the node lands it fails `Unsupported`, like the
 * engine (`crates/ether-controller/tests/roadmap_v3.rs`).
 */

import type { DeviceId, SampleZone } from "@/generated";
import { fail, type ReducerContext } from "../documentReducer";

export function setZones(ctx: ReducerContext, device: DeviceId, zones: SampleZone[]): void {
  void ctx;
  void device;
  void zones;
  fail("Unsupported", "multisampler zones are not implemented yet (multisampler)");
}
