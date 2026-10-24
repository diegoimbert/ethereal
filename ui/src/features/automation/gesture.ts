/**
 * Sending automation edits. A drag is one `LaneGesture`: every command it sends carries the
 * same gesture id (one undo step) and `Edit::EndGesture` closes it. While a command is in
 * flight only the latest pending one is kept, so fast pointer moves don't queue up.
 */

import type { Command } from "@/generated";
import { cmd, nextGestureId, type EngineTransport } from "@/transport";

const warn = (e: unknown) => console.warn("[ethereal] automation edit failed:", e);

export class LaneGesture {
  private readonly gesture = nextGestureId();
  private inFlight = false;
  private pending: { command: Command | (() => Command | null); after?: () => void } | null = null;
  private sent = false;
  private ended = false;
  private closed = false;

  constructor(private readonly transport: EngineTransport) {}

  /**
   * Send `command` (latest wins while one is in flight). `after` runs once it went through
   * (not when it was superseded).
   */
  update(command: Command | null, after?: () => void): void {
    if (!command || this.ended) return;
    if (this.inFlight) this.pending = { command, after };
    else this.dispatch(command, after);
  }

  /**
   * Like `update`, but the command is built when it is sent (`build` runs once per send, never
   * for a superseded call): for edits that depend on what the previous send did (draw mode
   * replaces the points its last send added).
   */
  updateWith(build: () => Command | null, after?: () => void): void {
    if (this.ended) return;
    if (this.inFlight) this.pending = { command: build, after };
    else this.dispatch(build, after);
  }

  /** Close the gesture once the last command is through. */
  end(): void {
    this.ended = true;
    if (!this.inFlight) this.close();
  }

  private dispatch(next: Command | (() => Command | null), after?: () => void): void {
    const command = typeof next === "function" ? next() : next;
    if (!command) {
      if (this.ended) this.close();
      return;
    }
    this.inFlight = true;
    this.sent = true;
    this.transport
      .send(command, { gesture: this.gesture })
      .then(() => after?.(), warn)
      .finally(() => {
        this.inFlight = false;
        const next = this.pending;
        this.pending = null;
        if (next) this.dispatch(next.command, next.after);
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
export function sendEdit(transport: EngineTransport, command: Command | null): Promise<void> {
  if (!command) return Promise.resolve();
  return transport.send(command).then(() => undefined, warn);
}
