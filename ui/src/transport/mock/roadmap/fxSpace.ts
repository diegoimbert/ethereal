/**
 * Mock of `Device::SetIr` and `Device::ListFactoryIrs` (v0.3, `fx-space`): the convolution
 * reverb's impulse response (`BuiltinDevice::ConvolutionReverb::ir`, one undo step) and the
 * factory IR list. Validates like the engine (`crates/ether-controller/src/fx_space`): the
 * device must be a convolution reverb, a factory id must exist, `Media` IRs must exist in
 * the project. The list is generated from Rust (`fxSpace.factoryIrs.json`, checked by
 * `crates/ether-devices/tests/fx_space.rs`). The mock engine makes no sound.
 */

import type { DeviceId, FactoryIr, IrSource, ReplyValue } from "@/generated";
import { fail, type ReducerContext } from "../documentReducer";
import factoryIrs from "./fxSpace.factoryIrs.json";

/** `ether_devices::fx_space::factory_irs()`. */
export const FACTORY_IRS: ReadonlyArray<FactoryIr> = factoryIrs as FactoryIr[];

/** `Device::SetIr` (document command). */
export function setIr(ctx: ReducerContext, device: DeviceId, ir: IrSource | null): void {
  const { tx } = ctx;
  const d = tx.get("Device", device) ?? fail("NotFound", `device ${device}`);
  if (d.kind.type !== "Builtin" || d.kind.device.type !== "ConvolutionReverb") {
    fail("InvalidArgument", `device ${d.id} is not a convolution reverb`);
  }
  if (ir?.type === "Factory" && !FACTORY_IRS.some((f) => f.id === ir.id)) {
    fail("NotFound", `factory impulse response "${ir.id}"`);
  }
  if (ir?.type === "Media" && !tx.get("Media", ir.media)) fail("NotFound", `media ${ir.media}`);
  tx.upsert("Device", { ...d, kind: { type: "Builtin", device: { type: "ConvolutionReverb", ir: ir ? { ...ir } : null } } });
}

/** `Device::ListFactoryIrs` (query) → `FactoryIrs`. */
export function listFactoryIrs(): ReplyValue {
  return { type: "FactoryIrs", irs: FACTORY_IRS.map((f) => ({ ...f })) };
}
