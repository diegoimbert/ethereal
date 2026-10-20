/**
 * Sending tempo edits. A drag is one `TempoGesture`: every command carries the same gesture
 * id (one undo step) and `Edit::EndGesture` closes it. While a command is in flight only
 * the latest pending one is kept, so fast pointer moves don't queue up.
 */

import { useContext } from "react";
import type { Command } from "@/generated";
import { dragEnd, type DragCancel } from "@/kit";
import { cmd, nextGestureId, TransportContext, type EngineTransport } from "@/transport";

const warn = (e: unknown) => console.warn("[ethereal] tempo edit failed:", e);

export class TempoGesture {
  private readonly gesture = nextGestureId();
  private inFlight = false;
  private pending: Command | null = null;
  private sent = false;
  private ended = false;
  private closed = false;
  private cancelled = false;

  constructor(private readonly transport: EngineTransport | null) {}

  /** Send `command` (latest wins while one is in flight). */
  update(command: Command): void {
    if (this.ended || !this.transport) return;
    if (this.inFlight) this.pending = command;
    else this.dispatch(command);
  }

  /** Close the gesture once the last command is through. */
  end(): void {
    this.ended = true;
    if (!this.inFlight) this.close();
  }

  /**
   * Escape: take back everything this gesture applied and leave no undo step
   * (`Edit::CancelGesture` once the last command is through; a queued one is dropped).
   */
  cancel(): void {
    if (this.ended) return;
    this.cancelled = true;
    this.pending = null;
    this.end();
  }

  private dispatch(command: Command): void {
    const transport = this.transport;
    if (!transport) return;
    this.inFlight = true;
    this.sent = true;
    transport
      .send(command, { gesture: this.gesture })
      .catch(warn)
      .finally(() => {
        this.inFlight = false;
        const next = this.pending;
        this.pending = null;
        if (next) this.dispatch(next);
        else if (this.ended) this.close();
      });
  }

  private close(): void {
    if (this.closed || !this.sent) return;
    this.closed = true;
    const type = this.cancelled ? "CancelGesture" : "EndGesture";
    this.transport?.send(cmd("Edit", { type, gesture: this.gesture })).catch(warn);
  }
}

/** The engine transport, or `null` outside a `TransportProvider` (edits are then no-ops). */
export function useTempoTransport(): EngineTransport | null {
  return useContext(TransportContext)?.transport ?? null;
}

/** Send a one-shot edit (its own undo step). */
export function sendEdit(transport: EngineTransport | null, command: Command): Promise<void> {
  if (!transport) return Promise.resolve();
  return transport.send(command).then(() => undefined, warn);
}

/**
 * Window-level pointer drag until release; `onMove` only after a small threshold. Returns
 * the drag's cancel (the same as a release), for `useDragScope`.
 */
export function trackDrag(
  start: { clientX: number; clientY: number },
  onMove: (e: PointerEvent) => void,
  onUp: (moved: boolean) => void,
  threshold = 3,
): DragCancel {
  let moved = false;
  const move = (e: PointerEvent) => {
    if (!moved && Math.hypot(e.clientX - start.clientX, e.clientY - start.clientY) < threshold) return;
    moved = true;
    onMove(e);
  };
  const up = dragEnd(() => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", up);
    onUp(moved);
  });
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", up);
  return up;
}
