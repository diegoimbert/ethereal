import type { PointerEvent } from "react";

export interface SplitterProps {
  /** "vertical": a column divider dragged left/right; "horizontal": a row divider dragged up/down. */
  orientation: "vertical" | "horizontal";
  size: number;
  /** Pixels the pane grows per pixel of pointer motion along the axis (1 or -1). */
  direction: 1 | -1;
  min: number;
  /** Upper bound, evaluated at drag start (e.g. from the window size). */
  max: () => number;
  onResize: (px: number) => void;
  /** Double-click restores this size. */
  reset: number;
  className?: string;
  label: string;
}

/** Drag handle between two shell panes. */
export function Splitter({ orientation, size, direction, min, max, onResize, reset, className, label }: SplitterProps) {
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const horizontal = orientation === "horizontal";
    const start = horizontal ? e.clientY : e.clientX;
    const hi = Math.max(min, max());
    const cursor = horizontal ? "ns-resize" : "ew-resize";
    const move = (ev: globalThis.PointerEvent) => {
      const d = (horizontal ? ev.clientY : ev.clientX) - start;
      onResize(Math.min(hi, Math.max(min, size + direction * d)));
    };
    const done = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", done);
      window.removeEventListener("pointercancel", done);
      document.body.style.removeProperty("cursor");
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", done);
    window.addEventListener("pointercancel", done);
    document.body.style.cursor = cursor;
  };
  return (
    <div
      className={`eth-splitter eth-splitter--${orientation}${className ? ` ${className}` : ""}`}
      role="separator"
      aria-orientation={orientation}
      aria-label={label}
      aria-valuenow={Math.round(size)}
      onPointerDown={onPointerDown}
      onDoubleClick={() => onResize(reset)}
    />
  );
}
