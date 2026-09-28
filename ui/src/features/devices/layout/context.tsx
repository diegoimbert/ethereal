/**
 * Render context of one device panel and the param binding every widget uses: the value
 * (document plain value, normalized, display text), writes (`Device::SetParam`, snapped to
 * `step`, one undo step per drag via the shared gesture sender) and reset.
 */

import { createContext, useContext, useMemo } from "react";
import type { Device, DeviceDescriptor, ParamId, ParamInfo } from "@/generated";
import { cmd } from "@/transport";
import type { GestureSender } from "../gesture";
import { formatValue, isBipolar, snapPlain, toNormalized, toPlain } from "./values";

export interface LayoutContextValue {
  device: Device;
  descriptor: DeviceDescriptor;
  params: ReadonlyMap<ParamId, ParamInfo>;
  sender: GestureSender;
}

export const LayoutContext = createContext<LayoutContextValue | null>(null);

export function useLayoutContext(): LayoutContextValue {
  const ctx = useContext(LayoutContext);
  if (!ctx) throw new Error("device layout widget rendered outside <DeviceLayoutView>");
  return ctx;
}

export function useLayoutContextValue(device: Device, descriptor: DeviceDescriptor, sender: GestureSender): LayoutContextValue {
  const params = useMemo(() => new Map(descriptor.params.map((p) => [p.id, p])), [descriptor]);
  return useMemo(() => ({ device, descriptor, params, sender }), [device, descriptor, params, sender]);
}

export interface ParamBinding {
  device: Device;
  info: ParamInfo;
  /** Document (base) plain value. */
  plain: number;
  normalized: number;
  /** Display text (labels / unit / step). */
  text: string;
  bipolar: boolean;
  /** Write a plain value (snapped; no-op when unchanged). */
  setPlain(plain: number): void;
  /** Write a normalized value (mapped through the scale, snapped). */
  setNormalized(normalized: number): void;
  /** Pointer drag started / ended: one undo gesture. */
  begin(): void;
  end(): void;
  /** Back to `ParamInfo.default`. */
  reset(): void;
}

export function bindParam(ctx: LayoutContextValue, info: ParamInfo): ParamBinding {
  const { device, sender } = ctx;
  const plain = device.params[info.id] ?? info.default;
  const setPlain = (value: number) => {
    const v = snapPlain(info, value);
    if (v === plain) return;
    void sender.send(cmd("Device", { type: "SetParam", device: device.id, param: info.id, value: v }));
  };
  return {
    device,
    info,
    plain,
    normalized: toNormalized(info, plain),
    text: formatValue(info, plain),
    bipolar: isBipolar(info),
    setPlain,
    setNormalized: (n) => setPlain(toPlain(info, n)),
    begin: sender.begin,
    end: sender.end,
    reset: () => setPlain(info.default),
  };
}

/** Binding of `param`, or `null` when the descriptor has no such param. */
export function useParam(param: ParamId | null | undefined): ParamBinding | null {
  const ctx = useLayoutContext();
  if (param == null) return null;
  const info = ctx.params.get(param);
  return info ? bindParam(ctx, info) : null;
}
