// OWNERSHIP: the `device-ui` node owns `ui/src/features/devices/layout/**` (the one shared
// device renderer + the widget catalog, CONTRACTS.md §12.4.2). `graphical-eq` owns `./eq/**`
// and registers its `EqCurve` here and in `Widget.tsx`.
//
// Device nodes never write panels: they ship a `DeviceLayout` in their descriptor and this
// renderer draws it with kit components and tokens. Seams for other nodes: `./seams.ts`.

export { DeviceLayoutView, type DeviceLayoutViewProps } from "./DeviceLayoutView";
export { WidgetView, type WidgetViewProps } from "./Widget";
export { bindParam, LayoutContext, useLayoutContext, useParam, type LayoutContextValue, type ParamBinding } from "./context";
export { ParamShell, paramMenu, showAutomation, useAutomated } from "./ParamShell";
export {
  boundParams,
  genericLayout,
  genericWidget,
  groupParams,
  referencedParams,
  resolveLayout,
  splitMainParams,
  type ResolvedLayout,
} from "./model";
export { formatValue, snapPlain, toNormalized, toPlain } from "./values";
export { Plot, GridLines, useBoxSize, type PlotDrag } from "./plot";
export { TypedFrame, ControlsRow } from "./widgets/typed";
export { ParamModulation, useDeviceAnalysis, type ParamModulationProps } from "./seams";
