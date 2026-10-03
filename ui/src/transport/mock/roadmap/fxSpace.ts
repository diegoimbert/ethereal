/**
 * Mock of `Device::SetIr` and `Device::ListFactoryIrs` (v0.3, contracts-4). Owned by
 * `fx-space`: the convolution reverb's impulse response
 * (`BuiltinDevice::ConvolutionReverb::ir`, one undo step; `Media` IRs must exist in the
 * project) and the factory IR list. The device itself inserts as a placeholder (descriptor
 * in `../devices/fxSpace.json`). Until the node lands both fail `Unsupported`, like the
 * engine (`crates/ether-controller/tests/roadmap_v4.rs`).
 */

import type { DeviceId, IrSource, ReplyValue } from "@/generated";
import { fail, type ReducerContext } from "../documentReducer";

/** `Device::SetIr` (document command). */
export function setIr(ctx: ReducerContext, device: DeviceId, ir: IrSource | null): void {
  void ctx;
  void device;
  void ir;
  fail("Unsupported", "impulse responses are not implemented yet (fx-space)");
}

/** `Device::ListFactoryIrs` (query) → `FactoryIrs`. */
export function listFactoryIrs(): ReplyValue {
  return fail("Unsupported", "factory impulse responses are not implemented yet (fx-space)");
}
