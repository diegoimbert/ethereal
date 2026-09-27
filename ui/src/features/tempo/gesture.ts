/**
 * Sending tempo edits. A drag is one `TempoGesture`: every command carries the same gesture
 * id (one undo step) and `Edit::EndGesture` closes it. While a command is in flight only
 * the latest pending one is kept, so fast pointer moves don't queue up.
 */

import type { Command } from "@/generated";
import { cmd, nextGestureId, type EngineTransport } from "@/transport";

const warn = (e: unknown) => console.warn("[ethereal] tempo edit failed:", e);

export class TempoGesture {
  private readonly gesture = nextGestureId();
  private inFlight = false;
  private pending: Command | null = null;
  private sent = false;
  private ended = false;
  private closed = false;

  constructor(private readonly transport: EngineTransport) {}

  /** Send `command` (latest wins while one is in flight). */
  update(command: Command): void {
    if (this.ended) return;
    if (this.inFlight) this.pending = command;
    else this.dispatch(command);
  }

  /** Close the gesture once the last command is through. */
  end(): void {
    this.ended = true;
    if (!this.inFlight) this.close();
  }

  private dispatch(command: Command): void {
    this.inFlight = true;
    this.sent = true;
    this.transport
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
    this.transport.send(cmd("Edit", { type: "EndGesture", gesture: this.gesture })).catch(warn);
  }
}

/** Send a one-shot edit (its own undo step). */
export function sendEdit(transport: EngineTransport, command: Command): Promise<void> {
  return transport.send(command).then(() => undefined, warn);
}

/** Window-level pointer drag until release; `onMove` only after a small threshold. */
export function trackDrag(
  start: { clientX: number; clientY: number },
  onMove: (e: PointerEvent) => void,
  onUp: (moved: boolean) => void,
  threshold = 3,
): void {
  let moved = false;
  const move = (e: PointerEvent) => {
    if (!moved && Math.hypot(e.clientX - start.clientX, e.clientY - start.clientY) < threshold) return;
    moved = true;
    onMove(e);
  };
  const up = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", up);
    onUp(moved);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", up);
}
