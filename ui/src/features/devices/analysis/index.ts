/**
 * The analysis channel's UI side (`fx-analysis`): the hook the shared renderer's data
 * widgets (Spectrum, Tuner, Meter, the EQ overlay) read through `layout/seams.ts`.
 */

import { useCallback, useContext, useEffect, useSyncExternalStore } from "react";
import type { AnalysisData, DeviceId } from "@/generated";
import { TransportContext } from "@/transport";
import { analysisStore } from "./store";

export { AnalysisStore, analysisStore } from "./store";

const EMPTY: ReadonlyArray<AnalysisData> = Object.freeze([]);
const noop = () => () => {};
const none = () => EMPTY;

/**
 * The latest analysis frame per kind (spectrum: per stage) of `device`, watching it
 * (`Analysis::Watch`) while the calling component is mounted and the page is visible.
 * Returns `[]` until the first frame (or outside a `TransportProvider`).
 */
export function useDeviceAnalysis(device: DeviceId): ReadonlyArray<AnalysisData> {
  const transport = useContext(TransportContext)?.transport ?? null;
  const store = transport ? analysisStore(transport) : null;
  useEffect(() => store?.watch(device), [store, device]);
  const subscribe = useCallback((cb: () => void) => (store ? store.subscribe(device, cb) : noop()), [store, device]);
  const snapshot = useCallback(() => (store ? store.frames(device) : none()), [store, device]);
  return useSyncExternalStore(subscribe, snapshot);
}
