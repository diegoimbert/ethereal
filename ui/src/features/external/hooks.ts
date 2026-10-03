/**
 * Hardware ports and latency measurement for the external devices' panel
 * (`external-instrument`): `External::ListPorts` / `PortsChanged`, and
 * `External::MeasureLatency` with its `LatencyMeasured` / `MeasureFailed` events.
 */

import { useCallback, useEffect, useState } from "react";
import type { DeviceId, HardwarePorts } from "@/generated";
import { cmd, useTransport } from "@/transport";

export interface PortsState {
  /** `null` until listed, or when the host has no hardware I/O. */
  ports: HardwarePorts | null;
  /** The host has no hardware I/O (the web build). */
  unsupported: boolean;
}

const codeOf = (e: unknown): string | undefined =>
  typeof e === "object" && e !== null && "code" in e ? String((e as { code: unknown }).code) : undefined;
const messageOf = (e: unknown): string =>
  typeof e === "object" && e !== null && "message" in e ? String((e as { message: unknown }).message) : String(e);

/** The host's hardware ports, listed on mount and kept current by `PortsChanged`. */
export function useHardwarePorts(): PortsState {
  const transport = useTransport();
  const [state, setState] = useState<PortsState>({ ports: null, unsupported: false });
  useEffect(() => {
    let active = true;
    transport
      .send(cmd("External", { type: "ListPorts" }))
      .then((reply) => {
        if (active && reply.type === "HardwarePorts") setState({ ports: reply.ports, unsupported: false });
      })
      .catch((e: unknown) => {
        if (active && codeOf(e) === "Unsupported") setState({ ports: null, unsupported: true });
      });
    const off = transport.onEvent((e) => {
      if (e.type === "External" && e.event.type === "PortsChanged") setState({ ports: e.event.ports, unsupported: false });
    });
    return () => {
      active = false;
      off();
    };
  }, [transport]);
  return state;
}

export type MeasureState =
  | { state: "idle" }
  | { state: "measuring" }
  | { state: "done"; latencyMs: number }
  | { state: "failed"; message: string };

/** Measure `device`'s round trip; the result sets its `Latency` param (one undo step). */
export function useMeasureLatency(device: DeviceId): [MeasureState, () => void] {
  const transport = useTransport();
  const [state, setState] = useState<MeasureState>({ state: "idle" });
  useEffect(
    () =>
      transport.onEvent((e) => {
        if (e.type !== "External" || !("device" in e.event) || e.event.device !== device) return;
        if (e.event.type === "LatencyMeasured") setState({ state: "done", latencyMs: e.event.latency_ms });
        else if (e.event.type === "MeasureFailed") setState({ state: "failed", message: e.event.message });
      }),
    [transport, device],
  );
  const measure = useCallback(() => {
    setState({ state: "measuring" });
    transport.send(cmd("External", { type: "MeasureLatency", device })).catch((e: unknown) => {
      setState({ state: "failed", message: messageOf(e) });
    });
  }, [transport, device]);
  return [state, measure];
}
