// Shared UI primitives. Owned by `design-system` (docs/DESIGN-SYSTEM.md); every component reads
// only design tokens (ui/src/theme). Feature code should build on these + tokens only.
// The UI font (tokens `font.ui`), bundled so it works offline.
import "@fontsource-variable/inter";
import "./theme.css";
import "./kit.css";
import { initTheme } from "../theme";
import { initMotionLevel } from "@/timeline/motion";

initTheme();
// The animation level (Settings > Appearance). Tests keep motion off (vitest.setup.ts).
if (import.meta.env.MODE !== "test") initMotionLevel();

export { Button, type ButtonProps } from "./Button";
export { AttachedButton, type AttachedButtonProps, type ButtonAttachment } from "./AttachedButton";
export { ConfirmDialog } from "./confirm";
export { DismissibleError, type DismissibleErrorProps } from "./DismissibleError";
export { useConfirm, type ConfirmRequest } from "./confirmStore";
export { ContextMenuHost } from "./ContextMenu";
export { TitleTooltipHost } from "./TitleTooltips";
export { MOD_KEY, openContextMenu, useContextMenuStore, type ContextMenuEntry, type ContextMenuItem } from "./contextMenuStore";
export { setDragCursor } from "./dragCursor";
export { Fader, type FaderProps } from "./Fader";
export {
  closeTopFloatingWindow,
  FloatingWindow,
  resetFloatingWindows,
  useFloatingWindows,
  windowRect,
  type FloatingWindowProps,
  type WindowRect,
} from "./floatingWindows";
export { NumberField, TextInput, type NumberFieldProps, type TextInputProps } from "./fields";
export { SEARCH_OVER, Select, type SelectOption, type SelectProps } from "./Select";
export { VIRTUAL_MAX_ROWS, VirtualList, type VirtualListProps } from "./VirtualList";
export { IconButton, type IconButtonProps } from "./IconButton";
export { Logo, type LogoProps } from "@/brand/Logo";
export { Knob, type KnobProps } from "./Knob";
export { Meter, type MeterProps } from "./Meter";
export { meterPosition } from "./meterScale";
export {
  AmbientBlobs,
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
export { disabledProps, menuItems, menuKeyDown, useFocusFirstItem } from "./menuKeyboard";
export { Panel, type PanelProps } from "./Panel";
export { ReadOnlySurface } from "./ReadOnlySurface";
export { useDisabled } from "./readOnly";
export { Tabs, type TabItem, type TabsProps } from "./Tabs";
export { dragEnd, useDragScope, useScopedDrag, type DragCancel } from "./dragScope";
export * from "./theme";
export { Toast, ToastStack, type ToastProps } from "./Toast";
export { Toggle, type ToggleProps } from "./Toggle";
export { clamp01, useVerticalDrag, type VerticalDragOptions } from "./useVerticalDrag";
export { useMediaQuery } from "./useMediaQuery";
export { resolveTone, SIZES, STATUS_TONES, TONES, type Size, type StatusTone, type Tone } from "./variants";
