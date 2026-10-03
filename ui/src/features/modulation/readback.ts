/**
 * Live modulated values (`AnalysisData::Modulation` readback, ≤ 30 Hz, CONTRACTS.md
 * §12.4.3): the engine's normalized effective value of each modulated param of a watched
 * device. Watching is refcounted per transport and device (`Analysis::Watch` while any
 * ring of the device is mounted).
 */

import { useEffect, useState } from "react";
import type { DeviceId, ParamId } from "@/generated";
import { cmd, useTransport, type EngineTransport } from "@/transport";

const watches = new WeakMap<EngineTransport, Map<DeviceId, number>>();

function watch(transport: EngineTransport, device: DeviceId): () => void {
  let counts = watches.get(transport);
  if (!counts) watches.set(transport, (counts = new Map()));
  const n = counts.get(device) ?? 0;
  counts.set(device, n + 1);
  if (n === 0) void transport.send(cmd("Analysis", { type: "Watch", device })).catch(() => undefined);
  return () => {
    const left = (counts.get(device) ?? 1) - 1;
    if (left > 0) {
      counts.set(device, left);
      return;
    }
    counts.delete(device);
    void transport.send(cmd("Analysis", { type: "Unwatch", device })).catch(() => undefined);
  };
}

/**
 * The live effective value (normalized) of `device`'s `param`, or `null` until the engine
 * reports one. Only watches while `active`.
 */
export function useModulatedValue(device: DeviceId, param: ParamId, active: boolean): number | null {
  const transport = useTransport();
  const [value, setValue] = useState<number | null>(null);
  useEffect(() => {
    if (!active) return;
    const unwatch = watch(transport, device);
    const off = transport.onEvent((e) => {
      if (e.type !== "Analysis" || e.event.type !== "Frame" || e.event.device !== device) return;
      const data = e.event.data;
      if (data.type !== "Modulation") return;
      const v = data.values.find((x) => x.param === param);
      if (v) setValue(v.value);
    });
    return () => {
      off();
      unwatch();
      setValue(null);
    };
  }, [transport, device, param, active]);
  return value;
}
