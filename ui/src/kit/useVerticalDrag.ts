import { useCallback, useRef } from "react";
import type { KeyboardEvent, PointerEvent } from "react";

export const clamp01 = (v: number): number => (v < 0 ? 0 : v > 1 ? 1 : v);

export interface VerticalDragOptions {
  value: number;
  onChange?: ((value: number) => void) | undefined;
  /**
   * Value change per pixel dragged upward. Shift divides it by 10 (fine mode). A function is
   * called with the dragged element at drag start (e.g. `1 / el.clientHeight` for a fader).
   */
  sensitivity: number | ((el: HTMLElement) => number);
  /** Double-click resets to this value. */
  defaultValue?: number | undefined;
  /** Called when a pointer drag starts. Open an undo gesture here. */
  onChangeStart?: (() => void) | undefined;
  /** Called when a pointer drag ends or is cancelled. Close the gesture here. */
  onChangeEnd?: (() => void) | undefined;
}

/**
 * Pointer + keyboard handlers for a vertical 0..1 control (knobs, faders).
 * Drag up to increase; Shift = fine; arrows/PageUp/PageDown/Home/End on focus.
 */
export function useVerticalDrag({
  value,
  onChange,
  sensitivity,
  defaultValue,
  onChangeStart,
  onChangeEnd,
}: VerticalDragOptions) {
  const drag = useRef<{ startY: number; startValue: number; sensitivity: number } | null>(null);

  const onPointerDown = useCallback(
    (e: PointerEvent<HTMLElement>) => {
      if (e.button !== 0) return;
      e.currentTarget.setPointerCapture(e.pointerId);
      const sens = typeof sensitivity === "function" ? sensitivity(e.currentTarget) : sensitivity;
      drag.current = { startY: e.clientY, startValue: value, sensitivity: Number.isFinite(sens) ? sens : 0.01 };
      if (onChange) onChangeStart?.();
      e.preventDefault();
    },
    [value, onChange, onChangeStart, sensitivity],
  );

  const onPointerMove = useCallback(
    (e: PointerEvent<HTMLElement>) => {
      const d = drag.current;
      if (!d || !onChange) return;
      const s = e.shiftKey ? d.sensitivity / 10 : d.sensitivity;
      const next = clamp01(d.startValue + (d.startY - e.clientY) * s);
      if (next !== value) onChange(next);
    },
    [onChange, value],
  );

  const onPointerUp = useCallback(
    (e: PointerEvent<HTMLElement>) => {
      const wasDragging = drag.current !== null;
      drag.current = null;
      if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
      if (wasDragging && onChange) onChangeEnd?.();
    },
    [onChange, onChangeEnd],
  );

  const onDoubleClick = useCallback(() => {
    if (defaultValue !== undefined) onChange?.(clamp01(defaultValue));
  }, [defaultValue, onChange]);

  const onKeyDown = useCallback(
    (e: KeyboardEvent<HTMLElement>) => {
      const step = e.shiftKey ? 0.001 : 0.01;
      let next: number | null = null;
      switch (e.key) {
        case "ArrowUp":
        case "ArrowRight":
          next = value + step;
          break;
        case "ArrowDown":
        case "ArrowLeft":
          next = value - step;
          break;
        case "PageUp":
          next = value + 0.1;
          break;
        case "PageDown":
          next = value - 0.1;
          break;
        case "Home":
          next = 0;
          break;
        case "End":
          next = 1;
          break;
      }
      if (next !== null) {
        e.preventDefault();
        onChange?.(clamp01(next));
      }
    },
    [onChange, value],
  );

  return { onPointerDown, onPointerMove, onPointerUp, onPointerCancel: onPointerUp, onDoubleClick, onKeyDown };
}
