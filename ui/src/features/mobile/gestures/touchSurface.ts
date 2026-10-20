/**
 * Binds the gesture recognizer to an element's touches. Mouse and pen input never come
 * here; the desktop handlers see it as before.
 *
 * Every touch that starts on the surface gets one of three policies (`policy`):
 *   - "native": the browser handles it (text fields: the caret, the keyboard);
 *   - "direct": the element's own pointer handlers handle it unchanged (the ruler scrubs, a
 *     fader drags, a knob turns: all already follow a pressed pointer);
 *   - "gesture" (default): recognized here. The press never reaches the desktop handlers, which
 *     would start a marquee or a drag at once. Instead:
 *       - tap: replayed as a mouse click on the pressed element, so every desktop click works
 *         (select a clip or a track, a header button, the insert marker); a double-tap also gets
 *         a `dblclick` (and its second press counts as a double press);
 *       - long-press then release: a `contextmenu` there, so every desktop context menu opens;
 *       - pan: `onPanStart` decides. "view" moves the view (`onPan`, then a fling from
 *         `onPanEnd`). "replay" replays a mouse press where the finger first touched, and
 *         "handled" means the handler started a desktop drag itself (a clip move or trim): either
 *         way the finger's moves and release then reach the desktop drag as mouse events, offset
 *         by how far `autoScroll` scrolled the content under it near the edges;
 *       - pinch: `onPinch`. A second finger during a drag cancels it (`pointercancel`).
 * The browser's own click, double-click and context menu from these touches are swallowed (it
 * still sends them after a tap, and Android opens a context menu on a long press).
 */
import { GestureRecognizer, type Gesture, type Point } from "./recognizer";
import { haptic, LONG_PRESS_MS } from "./constants";

export type TouchPolicy = "native" | "direct" | "gesture";
export type PanMode = "view" | "replay" | "handled";
/** Modifier keys a replayed press (and its drag) carries: the desktop gesture they select (Shift-drag: a time range; Ctrl-click: toggle). */
export interface Mods {
  shiftKey?: boolean;
  ctrlKey?: boolean;
  metaKey?: boolean;
  altKey?: boolean;
}

export interface TouchSurfaceHandlers {
  policy?(target: Element, p: Point): TouchPolicy;
  /** A finger touched the surface (stop a fling). */
  onTouchStart?(): void;
  /** How a one-finger drag starting on `target` is handled (default "view"). */
  onPanStart?(g: Extract<Gesture, { type: "pan-start" }>, target: Element): PanMode | { mode: "replay"; mods: Mods };
  onPan?(dx: number, dy: number): void;
  onPanEnd?(vx: number, vy: number): void;
  onPinchStart?(g: Extract<Gesture, { type: "pinch-start" }>): void;
  onPinch?(g: Extract<Gesture, { type: "pinch" }>): void;
  onPinchEnd?(): void;
  /** The finger is held (haptic already given): lift feedback. */
  onLongPress?(g: Extract<Gesture, { type: "longpress" }>, target: Element): void;
  /** Override the replayed click of a tap (return true when handled). */
  onTap?(g: Extract<Gesture, { type: "tap" }>, target: Element): boolean;
  /** A drag ("replay" / "handled") moved: the finger's point (feedback above it). */
  onDragMove?(p: Point): void;
  /** The drag ended (dropped or cancelled). */
  onDragEnd?(): void;
  /**
   * Each frame of a drag: scroll near the edges for a finger at `p`, `dt` ms since the last
   * frame. Returns the horizontal content shift in px (the timeline scrolled under the finger:
   * added to the pointer the drag sees) and whether anything scrolled.
   */
  autoScroll?(p: Point, dt: number): { dx: number; moved: boolean };
}

/** The pointer id of replayed mouse input (a mouse's usual id). */
const REPLAY_POINTER_ID = 1;
/** The browser's own click / context menu can follow a touch by this long (ms). */
const NATIVE_FOLLOW_UP_MS = 600;
/** ... and lands this close (px) to where the finger lifted. */
const NATIVE_FOLLOW_UP_PX = 24;

/** Dispatch a mouse-like pointer or mouse event at a client point. */
export function replay(target: EventTarget, type: string, p: Point, init: { button?: number; buttons?: number; detail?: number; mods?: Mods } = {}): boolean {
  const opts = {
    bubbles: true,
    cancelable: true,
    composed: true,
    clientX: p.x,
    clientY: p.y,
    screenX: p.x,
    screenY: p.y,
    button: init.button ?? 0,
    buttons: init.buttons ?? 0,
    detail: init.detail ?? 0,
    shiftKey: init.mods?.shiftKey ?? false,
    ctrlKey: init.mods?.ctrlKey ?? false,
    metaKey: init.mods?.metaKey ?? false,
    altKey: init.mods?.altKey ?? false,
    view: window,
  };
  let ev: Event;
  if (type.startsWith("pointer") && typeof PointerEvent === "function") {
    ev = new PointerEvent(type, {
      ...opts,
      pointerId: REPLAY_POINTER_ID,
      pointerType: "mouse",
      isPrimary: true,
      width: 1,
      height: 1,
      pressure: opts.buttons ? 0.5 : 0,
    });
  } else {
    ev = new MouseEvent(type, opts);
    if (type.startsWith("pointer")) Object.assign(ev, { pointerId: REPLAY_POINTER_ID, pointerType: "mouse", isPrimary: true });
  }
  return target.dispatchEvent(ev);
}

/** A tap as the desktop sees a click: press, release, click (and `dblclick` for a double-tap). */
export function replayClick(target: Element, p: Point, count: 1 | 2, mods: Mods = {}): void {
  replay(target, "pointerdown", p, { buttons: 1, detail: count, mods });
  replay(target, "mousedown", p, { buttons: 1, detail: count, mods });
  // The release goes where the desktop's drag listeners are (window), from the pressed element.
  replay(target.isConnected ? target : window, "pointerup", p, { detail: count, mods });
  replay(target.isConnected ? target : window, "mouseup", p, { detail: count, mods });
  if (target.isConnected) replay(target, "click", p, { detail: count, mods });
  if (count === 2 && target.isConnected) replay(target, "dblclick", p, { detail: 2, mods });
}

export function bindTouchSurface(el: HTMLElement, h: TouchSurfaceHandlers): () => void {
  /** The touches this surface follows (gesture policy). */
  const ids = new Set<number>();
  let target: Element | null = null;
  let panMode: PanMode = "view";
  /** A drag ("replay" / "handled") is going on: the finger's last point and the content shift. */
  let drag: { finger: Point; offsetX: number; frame: number; last: number } | null = null;
  /** The modifier keys the replayed drag carries. */
  let mods: Mods = {};
  let timer: ReturnType<typeof setTimeout> | null = null;
  /** Where and when a followed touch last lifted. */
  let lastUp = { x: 0, y: 0, t: -Infinity };

  const clearTimer = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  };
  const dragPoint = (): Point => (drag ? { x: drag.finger.x + drag.offsetX, y: drag.finger.y } : { x: 0, y: 0 });

  const startDrag = (finger: Point) => {
    drag = { finger, offsetX: 0, frame: 0, last: performance.now() };
    if (h.autoScroll) drag.frame = requestAnimationFrame(tick);
  };
  const tick = (now: number) => {
    const d = drag;
    if (!d || !h.autoScroll) return;
    const dt = Math.min(64, Math.max(0, now - d.last));
    d.last = now;
    const s = h.autoScroll(d.finger, dt);
    d.offsetX += s.dx;
    if (s.moved) replay(window, "pointermove", dragPoint(), { buttons: 1, mods });
    d.frame = requestAnimationFrame(tick);
  };
  const endDrag = () => {
    if (!drag) return;
    cancelAnimationFrame(drag.frame);
    drag = null;
    panMode = "view";
    h.onDragEnd?.();
  };

  const onGesture = (g: Gesture) => {
    const t = target;
    switch (g.type) {
      case "tap":
        if (t && !h.onTap?.(g, t)) replayClick(t, g, g.count);
        break;
      case "longpress":
        haptic();
        if (t) h.onLongPress?.(g, t);
        break;
      case "longpress-up":
        if (t?.isConnected) replay(t, "contextmenu", g, { button: 2 });
        break;
      case "pan-start":
        {
          const chosen = t ? (h.onPanStart?.(g, t) ?? "view") : "view";
          panMode = typeof chosen === "string" ? chosen : chosen.mode;
          mods = typeof chosen === "string" ? {} : chosen.mods;
        }
        if (panMode === "replay" && t) replay(t, "pointerdown", { x: g.startX, y: g.startY }, { buttons: 1, mods });
        if (panMode !== "view") startDrag({ x: g.x, y: g.y });
        break;
      case "pan":
        if (panMode === "view") h.onPan?.(g.dx, g.dy);
        break;
      case "pan-end":
        if (panMode === "view") h.onPanEnd?.(g.vx, g.vy);
        break;
      case "cancel":
        // A second finger: a desktop drag drops its preview.
        if (panMode !== "view") {
          replay(window, "pointercancel", dragPoint());
          endDrag();
        }
        break;
      case "pinch-start":
        h.onPinchStart?.(g);
        break;
      case "pinch":
        h.onPinch?.(g);
        break;
      case "pinch-end":
        h.onPinchEnd?.();
        break;
    }
  };
  const recognizer = new GestureRecognizer(onGesture);

  const onDown = (e: PointerEvent) => {
    if (e.pointerType !== "touch") return;
    const p = { x: e.clientX, y: e.clientY };
    if (ids.size === 0) {
      const t = e.target instanceof Element ? e.target : null;
      if (!t) return;
      const policy = h.policy?.(t, p) ?? "gesture";
      if (policy !== "gesture") return;
      target = t;
      panMode = "view";
      h.onTouchStart?.();
    }
    e.preventDefault();
    e.stopPropagation();
    ids.add(e.pointerId);
    recognizer.down(e.pointerId, p, e.timeStamp);
    clearTimer();
    if (recognizer.waitingForLongPress) timer = setTimeout(() => recognizer.timer(performance.now()), LONG_PRESS_MS);
  };
  const onMove = (e: PointerEvent) => {
    if (e.pointerType !== "touch" || !ids.has(e.pointerId)) return;
    recognizer.move(e.pointerId, { x: e.clientX, y: e.clientY }, e.timeStamp);
    if (!recognizer.waitingForLongPress) clearTimer();
    if (drag && ids.size === 1) {
      // The desktop drag sees the finger as a mouse, shifted by the auto-scroll.
      e.stopPropagation();
      drag.finger = { x: e.clientX, y: e.clientY };
      replay(window, "pointermove", dragPoint(), { buttons: 1, mods });
      h.onDragMove?.(drag.finger);
    }
  };
  const onUp = (e: PointerEvent) => {
    if (e.pointerType !== "touch" || !ids.has(e.pointerId)) return;
    ids.delete(e.pointerId);
    lastUp = { x: e.clientX, y: e.clientY, t: e.timeStamp };
    clearTimer();
    if (drag) {
      e.stopPropagation();
      drag.finger = { x: e.clientX, y: e.clientY };
      replay(window, "pointerup", dragPoint(), { mods });
      endDrag();
    }
    recognizer.up(e.pointerId, { x: e.clientX, y: e.clientY }, e.timeStamp);
  };
  const onCancel = (e: PointerEvent) => {
    if (e.pointerType !== "touch" || !ids.has(e.pointerId)) return;
    ids.clear();
    clearTimer();
    if (drag) {
      e.stopPropagation();
      replay(window, "pointercancel", dragPoint());
      endDrag();
    }
    recognizer.cancel();
  };
  /**
   * The browser's own follow-ups of a followed touch: Android's long-press menu (while the
   * finger is down) and the click after a tap, wherever it lands (a tap that opened a sheet must
   * not click its scrim), but only just after and where the finger lifted.
   */
  const swallow = (e: Event) => {
    if (!e.isTrusted) return;
    const m = e as MouseEvent;
    const followUp = e.timeStamp - lastUp.t < NATIVE_FOLLOW_UP_MS && Math.hypot(m.clientX - lastUp.x, m.clientY - lastUp.y) <= NATIVE_FOLLOW_UP_PX;
    if (ids.size > 0 || followUp) {
      e.preventDefault();
      e.stopPropagation();
    }
  };

  /** A press elsewhere (a button between two taps here) breaks a double-tap. */
  const onDownElsewhere = (e: PointerEvent) => {
    if (!(e.target instanceof Node && el.contains(e.target))) recognizer.forgetTap();
  };

  el.addEventListener("pointerdown", onDown, { capture: true, passive: false });
  window.addEventListener("pointerdown", onDownElsewhere, true);
  window.addEventListener("pointermove", onMove, true);
  window.addEventListener("pointerup", onUp, true);
  window.addEventListener("pointercancel", onCancel, true);
  for (const type of ["click", "dblclick", "contextmenu"]) window.addEventListener(type, swallow, true);
  return () => {
    clearTimer();
    endDrag();
    el.removeEventListener("pointerdown", onDown, { capture: true });
    window.removeEventListener("pointerdown", onDownElsewhere, true);
    window.removeEventListener("pointermove", onMove, true);
    window.removeEventListener("pointerup", onUp, true);
    window.removeEventListener("pointercancel", onCancel, true);
    for (const type of ["click", "dblclick", "contextmenu"]) window.removeEventListener(type, swallow, true);
  };
}
