// OWNERSHIP: the `racks-modulation` node owns `ui/src/features/modulation/**`.
// `ParamModulation` is picked up by the shared device renderer (`features/devices/layout/
// seams.ts`, eager `import.meta.glob`): keep its name and props. `ModulatorsPanel` and
// `AddModulatorButton` are mounted by `features/devices/DeviceView`.
export { ParamModulation, type ParamModulationProps } from "./ParamModulation";
export { AddModulatorButton, ModulatorsPanel } from "./ModulatorsPanel";
export { addModulatorEntries, useModulatorKinds } from "./kinds";
export { canTarget, isChainRack, useModMapping } from "./model";
