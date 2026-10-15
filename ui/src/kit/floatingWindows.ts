// base-154: floating windows (one export line in the kit index).
export { FloatingWindow, type FloatingWindowProps } from "./FloatingWindow";
export { clampRect, resizeRect, RESIZE_EDGES, type ResizeEdge, type SizeLimits, type Viewport, type WindowRect } from "./floatingWindowGeometry";
export { closeTopFloatingWindow, resetFloatingWindows, useFloatingWindows, windowRect } from "./floatingWindowStore";
