/**
 * Pointer drags as undo gestures: every command sent during one drag carries the same
 * gesture id and `Edit::EndGesture` closes it on release, so a drag is one undo step.
 */

import type { Command, GestureId } from "@/generated";
import { cmd, nextGestureId, type EngineTransport } from "@/transport";

function report(p: Promise<unknown>): void {
  p.catch((e: unknown) => console.warn("[warp] command failed:", e));
}

/** Send one command as its own undo step, logging failures. */
export function sendOne(transport: EngineTransport, command: Command): void {
  report(transport.send(command));
}

/** An open gesture: `send` commands into it, `end` closes it (no-op if nothing was sent). */
export interface Gesture {
  send(command: Command): void;
  end(): void;
}

export function openGesture(transport: EngineTransport): Gesture {
  let gesture: GestureId | null = null;
  let last = "";
  return {
    send(command) {
      const key = JSON.stringify(command);
      if (key === last) return;
      last = key;
      gesture ??= nextGestureId();
      report(transport.send(command, { gesture }));
    },
    end() {
      if (gesture !== null) report(transport.send(cmd("Edit", { type: "EndGesture", gesture })));
      gesture = null;
      last = "";
    },
  };
}

/**
 * Window-level pointer drag from `e`: `move(dx, event)` returns the command for the current
 * pointer position (or null); all of them form one gesture.
 */
export function startDrag(
  transport: EngineTransport,
  e: { clientX: number },
  move: (dx: number, ev: PointerEvent) => Command | null,
): void {
  const x0 = e.clientX;
  const g = openGesture(transport);
  const onMove = (ev: PointerEvent) => {
    const c = move(ev.clientX - x0, ev);
    if (c) g.send(c);
  };
  const onUp = () => {
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
    window.removeEventListener("pointercancel", onUp);
    g.end();
  };
  window.addEventListener("pointermove", onMove);
  window.addEventListener("pointerup", onUp);
  window.addEventListener("pointercancel", onUp);
}
