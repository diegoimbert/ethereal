// Shared UI primitives. Owned by `foundation`; additions via BCR (ORCHESTRATION.md §6.1).
import "@fontsource-variable/inter";
import "./theme.css";
import "./kit.css";

export { Button, type ButtonProps } from "./Button";
export { ContextMenuHost } from "./ContextMenu";
export { MOD_KEY, openContextMenu, useContextMenuStore, type MenuEntry, type MenuItem } from "./contextMenuStore";
export { Fader, type FaderProps } from "./Fader";
export { Knob, type KnobProps } from "./Knob";
export { Meter, type MeterProps } from "./Meter";
export { meterPosition } from "./meterScale";
export { Panel, type PanelProps } from "./Panel";
export * from "./theme";
export { clamp01, useVerticalDrag, type VerticalDragOptions } from "./useVerticalDrag";
