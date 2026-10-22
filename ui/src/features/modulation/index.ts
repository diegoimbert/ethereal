// `ParamModulation` is picked up by the shared device renderer (`features/devices/layout/
// seams.ts`, eager `import.meta.glob`): keep its name and props. `ModulatorsPanel` and
// `AddModulatorButton` are mounted by `features/devices/DeviceView`.
export { ParamModulation, SwitchModulation, type ParamModulationProps } from "./ParamModulation";
export { AddModulatorButton, ModulatorsPanel } from "./ModulatorsPanel";
export { addModulatorEntries, useModulatorKinds } from "./kinds";
export { canTarget, DEVICE_ENABLED_PARAM, isChainRack, useModMapping } from "./model";
export { paramMenuEntries, switchMenuEntries } from "./macroMenu";
