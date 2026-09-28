// Shared UI primitives. Owned by `design-system` (docs/DESIGN-SYSTEM.md); every component reads
// only design tokens (ui/src/theme). Feature code should build on these + tokens only.
// The UI font (tokens `font.ui`), bundled so it works offline.
import "@fontsource-variable/inter";
import "./theme.css";
import "./kit.css";
import { initTheme } from "../theme";

initTheme();

export { Button, type ButtonProps } from "./Button";
export { ContextMenuHost } from "./ContextMenu";
export {
  MOD_KEY,
  openContextMenu,
  useContextMenuStore,
  type ContextMenuEntry,
  type ContextMenuItem,
} from "./contextMenuStore";
export { setDragCursor } from "./dragCursor";
export { Fader, type FaderProps } from "./Fader";
export { NumberField, TextInput, type NumberFieldProps, type TextInputProps } from "./fields";
export { Select, type SelectOption, type SelectProps } from "./Select";
export { IconButton, type IconButtonProps } from "./IconButton";
export { Knob, type KnobProps } from "./Knob";
export { Meter, type MeterProps } from "./Meter";
export { meterPosition } from "./meterScale";
export {
  Badge,
  Dialog,
  Menu,
  Popover,
  Tooltip,
  type BadgeProps,
  type DialogProps,
  type MenuEntry,
  type MenuProps,
  type Placement,
  type PopoverProps,
  type TooltipProps,
  type TriggerProps,
} from "./overlays";
export { Panel, type PanelProps } from "./Panel";
export { Tabs, type TabItem, type TabsProps } from "./Tabs";
export * from "./theme";
export { Toast, ToastStack, type ToastProps } from "./Toast";
export { Toggle, type ToggleProps } from "./Toggle";
export { clamp01, useVerticalDrag, type VerticalDragOptions } from "./useVerticalDrag";
export { resolveTone, SIZES, STATUS_TONES, TONES, type Size, type StatusTone, type Tone } from "./variants";
