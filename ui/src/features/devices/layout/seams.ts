/**
 * Extension seams of the shared renderer, filled by other nodes without touching this folder:
 *
 * - **Analysis** (`fx-analysis`): `ui/src/features/devices/analysis/index.ts` exports
 *   `useDeviceAnalysis(device: DeviceId): ReadonlyArray<AnalysisData>`, the latest frame per
 *   kind/stage of a device, watching it (`Analysis::Watch`) while a widget using it is
 *   mounted. Until that module exists data widgets get `[]` and draw their empty state.
 * - **Modulation** (`racks-modulation`): `ui/src/features/modulation/index.ts` exports
 *   `ParamModulation: ComponentType<ParamModulationProps>`, rendered inside every param
 *   widget (depth ring, drop target). Until then the slot is an empty placeholder element
 *   (`.eth-param__mod`) and every param widget carries `data-mod-target="<device>:<param>"`.
 *
 * Both are picked up with an eager `import.meta.glob`, so the file appearing is the whole
 * integration (no registration call, no edit here).
 */

import type { ComponentType } from "react";
import type { AnalysisData, Device, DeviceId, ParamInfo } from "@/generated";

export interface AnalysisModule {
  useDeviceAnalysis?: (device: DeviceId) => ReadonlyArray<AnalysisData>;
}

export interface ParamModulationProps {
  device: Device;
  info: ParamInfo;
  /** The base value (document), normalized 0..1. */
  normalized: number;
}

export interface ModulationModule {
  ParamModulation?: ComponentType<ParamModulationProps>;
}

const analysisModules = import.meta.glob<AnalysisModule>("../analysis/index.ts", { eager: true });
const modulationModules = import.meta.glob<ModulationModule>("../../modulation/index.ts", { eager: true });

const EMPTY: ReadonlyArray<AnalysisData> = [];
const noAnalysis = (_device: DeviceId): ReadonlyArray<AnalysisData> => EMPTY;

/** The analysis hook (`fx-analysis`), or a stub returning no frames. */
export const useDeviceAnalysis: (device: DeviceId) => ReadonlyArray<AnalysisData> =
  Object.values(analysisModules)[0]?.useDeviceAnalysis ?? noAnalysis;

/** The modulation decoration of param widgets (`racks-modulation`), if any. */
export const ParamModulation: ComponentType<ParamModulationProps> | null =
  Object.values(modulationModules)[0]?.ParamModulation ?? null;

/** Target id of a param for modulation drops (`<device>:<param>`). */
export function modTargetId(device: DeviceId, param: number): string {
  return `${device}:${param}`;
}
