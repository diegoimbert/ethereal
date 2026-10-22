// The shared device renderer + the widget catalog (CONTRACTS.md §12.4.2). Devices ship a
// `DeviceLayout` in their descriptor and this renderer draws it with kit components and
// tokens. Extension seams (modulation, analysis): `./seams.ts`.

export { DeviceLayoutView, type DeviceLayoutViewProps } from "./DeviceLayoutView";
export { WidgetView, type WidgetViewProps } from "./Widget";
export { bindParam, LayoutContext, useLayoutContext, useParam, type LayoutContextValue, type ParamBinding } from "./context";
export { ParamShell } from "./ParamShell";
export { paramMenu, paramTarget, showAutomation, showSwitchAutomation, switchMenu, switchTarget, useAutomated } from "./automation";
export {
  boundParams,
  cardParams,
  genericLayout,
  PARAM_CARD_CAP,
  genericWidget,
  macrosWidgetParams,
  referencedParams,
  resolveLayout,
  UNCAPPED_MAX,
  type ResolvedLayout,
} from "./model";
export { formatValue, snapPlain, toNormalized, toPlain } from "./values";
export { Plot, GridLines, type PlotDrag } from "./plot";
export { useBoxSize } from "./useBoxSize";
export { TypedFrame, ControlsRow } from "./widgets/typed";
export { ParamModulation, useDeviceAnalysis, type ParamModulationProps } from "./seams";
export { EqCurveWidget, magnitudeDb } from "./eq";
