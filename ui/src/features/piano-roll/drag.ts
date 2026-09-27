/**
 * Pointer drags as undo gestures: every command sent during one drag carries the same
 * gesture id and `Edit::EndGesture` closes it on release, so a drag is one undo step.
 */

import { useCallback } from "react";
import type { Command, GestureId } from "@/generated";
import { setDragCursor } from "@/kit";
import { cmd, nextGestureId, useTransport } from "@/transport";
import type { EngineTransport } from "@/transport";

/** Log a failed command; resolves to whether it succeeded (its patches are applied). */
function report(p: Promise<unknown>): Promise<boolean> {
  return p.then(
    () => true,
    (e: unknown) => {
      console.warn("[piano-roll] command failed:", e);
      return false;
    },
  );
}

/** Send one command as its own undo step, logging failures. */
export function useSend(): (command: Command) => Promise<boolean> {
  const transport = useTransport();
  return useCallback((command: Command) => report(transport.send(command)), [transport]);
}

export interface DragHandlers {
  /** Pointer moved by (dx, dy) px from the press point. Return the command to send, if any. */
  move(dx: number, dy: number, e: PointerEvent): Command | null;
  /** Called after release (the gesture is already closed). */
  end?(moved: boolean): void;
}

export interface DragOptions {
  /** Command sent on press, inside the gesture (e.g. adding the note being drawn). */
  initial?: Command;
  /** Called once `initial` has been applied. */
  afterInitial?: () => void;
  /** Pixels to travel before `move` is called (default 0). */
  threshold?: number;
  /** Cursor shown everywhere until release (e.g. "ew-resize" while resizing). */
  cursor?: string;
}

/**
 * Start a window-level pointer drag from `e`. Identical consecutive commands are sent
 * once; the gesture is closed only if something was sent.
 */
export function startDrag(
  transport: EngineTransport,
  e: { clientX: number; clientY: number },
  handlers: DragHandlers,
  opts: DragOptions = {},
): void {
  const x0 = e.clientX;
  const y0 = e.clientY;
  let gesture: GestureId | null = null;
  let last = "";
  let moved = false;
  const send = (command: Command): Promise<boolean> | null => {
    const key = JSON.stringify(command);
    if (key === last) return null;
    last = key;
    gesture ??= nextGestureId();
    return report(transport.send(command, { gesture }));
  };
  if (opts.initial) {
    void send(opts.initial)?.then((ok) => {
      if (ok) opts.afterInitial?.();
    });
  }

  const onMove = (ev: PointerEvent) => {
    const dx = ev.clientX - x0;
    const dy = ev.clientY - y0;
    if (!moved && Math.hypot(dx, dy) < (opts.threshold ?? 0)) return;
    moved = true;
    const command = handlers.move(dx, dy, ev);
    if (command) send(command);
  };
  if (opts.cursor) setDragCursor(opts.cursor);
  const onUp = () => {
    if (opts.cursor) setDragCursor(null);
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
    window.removeEventListener("pointercancel", onUp);
    if (gesture !== null) void report(transport.send(cmd("Edit", { type: "EndGesture", gesture })));
    handlers.end?.(moved);
  };
  window.addEventListener("pointermove", onMove);
  window.addEventListener("pointerup", onUp);
  window.addEventListener("pointercancel", onUp);
}
