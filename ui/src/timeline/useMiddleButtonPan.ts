import { useEffect, type RefObject } from "react";
import { inputSettings } from "./inputSettings";
import type { TimelineViewStore } from "./viewStore";

/**
 * Middle-button drag pans a timeline element: horizontal motion scrolls the view store,
 * vertical motion scrolls the element itself (a no-op when it doesn't scroll vertically).
 *
 * Listens in the capture phase and stops the event, so items and marquees underneath never
 * see the middle-button press. Also suppresses the browser's middle-click autoscroll.
 * Off when Settings > Input > Middle-button drag is "Off" (the press then reaches the items
 * underneath like any other; autoscroll stays suppressed).
 */
export function useMiddleButtonPan(ref: RefObject<HTMLElement | null>, view: TimelineViewStore): void {
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    let drag: { id: number; x: number; y: number } | null = null;

    const onDown = (e: PointerEvent) => {
      if (e.button !== 1 || inputSettings().middleButton !== "pan") return;
      e.preventDefault();
      e.stopPropagation();
      drag = { id: e.pointerId, x: e.clientX, y: e.clientY };
      el.setPointerCapture?.(e.pointerId);
      el.style.cursor = "grabbing";
    };
    const onMove = (e: PointerEvent) => {
      if (!drag || e.pointerId !== drag.id) return;
      const dx = e.clientX - drag.x;
      const dy = e.clientY - drag.y;
      drag.x = e.clientX;
      drag.y = e.clientY;
      if (dx) view.getState().scrollByPx(-dx);
      if (dy) el.scrollTop -= dy;
    };
    const onUp = (e: PointerEvent) => {
      if (!drag || e.pointerId !== drag.id) return;
      drag = null;
      el.style.cursor = "";
    };
    // Autoscroll starts on mousedown; auxclick would paste on Linux.
    const onMouseDown = (e: MouseEvent) => {
      if (e.button === 1) e.preventDefault();
    };
    const onAuxClick = (e: MouseEvent) => {
      if (e.button === 1) e.preventDefault();
    };

    el.addEventListener("pointerdown", onDown, { capture: true });
    el.addEventListener("pointermove", onMove);
    el.addEventListener("pointerup", onUp);
    el.addEventListener("pointercancel", onUp);
    el.addEventListener("lostpointercapture", onUp);
    el.addEventListener("mousedown", onMouseDown, { capture: true });
    el.addEventListener("auxclick", onAuxClick);
    return () => {
      el.removeEventListener("pointerdown", onDown, { capture: true });
      el.removeEventListener("pointermove", onMove);
      el.removeEventListener("pointerup", onUp);
      el.removeEventListener("pointercancel", onUp);
      el.removeEventListener("lostpointercapture", onUp);
      el.removeEventListener("mousedown", onMouseDown, { capture: true });
      el.removeEventListener("auxclick", onAuxClick);
      el.style.cursor = "";
    };
  }, [ref, view]);
}
