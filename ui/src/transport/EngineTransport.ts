/**
 * The engine connection every host implements (MockTransport, TauriTransport,
 * WasmTransport). Features only ever talk to the engine through this interface (via
 * `useTransport()`), and only read the document from the project store.
 *
 * Contract (all hosts):
 * - `connect()` resolves with the full current `Project`; the host then emits the current
 *   `TransportState` as `Event::Transport`. Subscribe to `onEvent` BEFORE calling it.
 * - `send()` resolves once the engine applied the command. Every `Event::Patch` caused by
 *   the command is delivered to `onEvent` listeners BEFORE the promise settles, so after
 *   `await send(...)` the project store is up to date.
 * - On `ReplyResult::Err` the promise rejects with `CommandFailedError` (`.code`,
 *   `.message`); the document is unchanged (commands are all-or-nothing).
 * - Entity ids are generated client-side (`newId()`) and passed in the command, so the UI
 *   knows the id of what it creates without waiting for a reply.
 * - Continuous edits (fader drags, note drags) pass the same `gesture` id on every send
 *   and finish with `Edit::EndGesture { gesture }`; the engine merges them into ONE undo
 *   step. Get ids from `nextGestureId()`.
 */

import type { Command, CommandError, ErrorCode, Event, GestureId, MeterFrame, PlayheadFrame, Project, ReplyValue } from "@/generated";

export type Unsubscribe = () => void;

export interface SendOptions {
  /** Merge this command into the current undo step of the same gesture. */
  gesture?: GestureId;
}

export interface EngineTransport {
  readonly kind: "mock" | "tauri" | "wasm";
  /** Connect and return the full current project. */
  connect(): Promise<Project>;
  /**
   * Send a command; resolves with the reply value, rejects with a `CommandFailedError`
   * (wrapping `CommandError`) on Err. Patches for the command are delivered to event
   * listeners BEFORE the promise resolves.
   */
  send(command: Command, opts?: SendOptions): Promise<ReplyValue>;
  /** Low-rate pushed events (ProjectLoaded, Patch, Transport, Plugin, ...). */
  onEvent(listener: (event: Event) => void): Unsubscribe;
  /** High-rate streams. */
  subscribePlayhead(listener: (frame: PlayheadFrame) => void): Unsubscribe;
  subscribeMeters(listener: (frame: MeterFrame) => void): Unsubscribe;
  dispose(): void;
}

/** A command the engine answered with `ReplyResult::Err`. */
export class CommandFailedError extends Error {
  readonly error: CommandError;
  /** The failed command, when known (for diagnostics). */
  readonly command: Command | undefined;

  constructor(error: CommandError, command?: Command) {
    super(`${error.code}: ${error.message}`);
    this.name = "CommandFailedError";
    this.error = error;
    this.command = command;
  }

  get code(): ErrorCode {
    return this.error.code;
  }
}

export function isCommandFailed(e: unknown, code?: ErrorCode): e is CommandFailedError {
  return e instanceof CommandFailedError && (code === undefined || e.code === code);
}

/**
 * Minimal synchronous event emitter. Listeners are called in subscription order; a
 * listener that throws doesn't prevent the others from running (the error is reported
 * asynchronously so it still shows up in the console / test runner).
 */
export class Emitter<T> {
  private listeners = new Set<(value: T) => void>();

  on(listener: (value: T) => void): Unsubscribe {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  emit(value: T): void {
    for (const l of [...this.listeners]) {
      try {
        l(value);
      } catch (e) {
        queueMicrotask(() => {
          throw e;
        });
      }
    }
  }

  get size(): number {
    return this.listeners.size;
  }

  clear(): void {
    this.listeners.clear();
  }
}
