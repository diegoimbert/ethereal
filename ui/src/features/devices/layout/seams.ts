/**
 * Extension seams of the shared renderer, filled from outside this folder:
 *
 * - **Analysis**: `ui/src/features/devices/analysis/index.ts` exports
 *   `useDeviceAnalysis(device: DeviceId): ReadonlyArray<AnalysisData>`, the latest frame per
 *   kind/stage of a device, watching it (`Analysis::Watch`) while a widget using it is
 *   mounted. Without that module data widgets get `[]` and draw their empty state.
 * - **Modulation**: `ui/src/features/modulation/index.ts` exports
 *   `ParamModulation: ComponentType<ParamModulationProps>`, rendered inside every param
 *   widget (depth ring, drop target). Without it the slot is an empty element
 *   (`.eth-param__mod`); every param widget carries `data-mod-target="<device>:<param>"`.
 *
 * Both are picked up with an eager `import.meta.glob` (no registration call), which keeps
 * the renderer free of imports from those features.
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
export const useDeviceAnalysis: (device: DeviceId) => ReadonlyArray<AnalysisData> = Object.values(analysisModules)[0]?.useDeviceAnalysis ?? noAnalysis;

/** The modulation decoration of param widgets (`racks-modulation`), if any. */
export const ParamModulation: ComponentType<ParamModulationProps> | null = Object.values(modulationModules)[0]?.ParamModulation ?? null;

/** Target id of a param for modulation drops (`<device>:<param>`). */
export function modTargetId(device: DeviceId, param: number): string {
  return `${device}:${param}`;
}
