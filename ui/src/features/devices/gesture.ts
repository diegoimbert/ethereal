/**
 * Continuous-control plumbing shared by the track controls (header, inspector) and the device chain.
 *
 * A drag on a knob/fader is one undo step: every command sent while the pointer is down
 * carries the same `gesture` id, and `Edit::EndGesture` closes it on pointer up. Wire
 * `begin`/`end` to the kit controls' `onChangeStart`/`onChangeEnd`. Changes made outside a
 * drag (keyboard, double-click reset) are sent without a gesture, one undo step each.
 *
 * During a drag, `Device::SetParam`s are coalesced to one per (device, param) per animation
 * frame, last value wins (perf-plugins, audit #4): pointer events can outpace the display,
 * and each command is a transaction, a patch and a collab message. `end` sends what is
 * pending before closing the gesture.
 */

import { useCallback, useMemo, useRef } from "react";
import type { Command, GestureId, ReplyValue } from "@/generated";
import { cmd, nextGestureId, useTransport } from "@/transport";

/** Log a failed command (the document is unchanged on failure; nothing to roll back). */
export function reportFailure(p: Promise<ReplyValue>): Promise<ReplyValue | undefined> {
  return p.catch((e: unknown) => {
    console.warn("[ethereal] command failed:", e);
    return undefined;
  });
}

/** `send` that logs failures instead of rejecting. */
export function useSend(): (command: Command) => Promise<ReplyValue | undefined> {
  const transport = useTransport();
  return useCallback((command: Command) => reportFailure(transport.send(command)), [transport]);
}

export interface GestureSender {
  /** Send `command`, inside the current drag's gesture if a drag is in progress. */
  send(command: Command): Promise<ReplyValue | undefined>;
  /** Start of a pointer drag (`onChangeStart`). */
  begin(): void;
  /** End of a pointer drag (`onChangeEnd`): closes the gesture if anything was sent. */
  end(): void;
  /** True between `begin` and `end`. */
  dragging(): boolean;
}

/** Coalescing key of a command sent during a drag (`null` = sent at once). */
function coalesceKey(command: Command): string | null {
  return command.domain === "Device" && command.command.type === "SetParam" ? `${command.command.device}:${command.command.param}` : null;
}

interface Pending {
  command: Command;
  waiters: ((reply: ReplyValue | undefined) => void)[];
}

const nextFrame = (f: () => void): (() => void) => {
  if (typeof requestAnimationFrame === "function") {
    const h = requestAnimationFrame(f);
    return () => cancelAnimationFrame(h);
  }
  const h = setTimeout(f, 16);
  return () => clearTimeout(h);
};

export function useGestureSender(): GestureSender {
  const transport = useTransport();
  const state = useRef<{ dragging: boolean; gesture: GestureId | null }>({ dragging: false, gesture: null });
  const queue = useRef<{ pending: Map<string, Pending>; cancel: (() => void) | null }>({ pending: new Map(), cancel: null });
  return useMemo<GestureSender>(() => {
    /** Send the coalesced commands now (inside `gesture`). */
    const flush = (gesture: GestureId | null) => {
      const q = queue.current;
      q.cancel?.();
      q.cancel = null;
      const pending = [...q.pending.values()];
      q.pending.clear();
      for (const p of pending) {
        void reportFailure(transport.send(p.command, gesture === null ? undefined : { gesture })).then((reply) => {
          for (const w of p.waiters) w(reply);
        });
      }
    };
    return {
      send(command) {
        const s = state.current;
        if (!s.dragging) return reportFailure(transport.send(command));
        if (s.gesture === null) s.gesture = nextGestureId();
        const gesture = s.gesture;
        const key = coalesceKey(command);
        if (key === null) {
          // Keep the order: anything coalesced so far goes first.
          flush(gesture);
          return reportFailure(transport.send(command, { gesture }));
        }
        const q = queue.current;
        return new Promise((resolve) => {
          const p = q.pending.get(key);
          if (p) {
            p.command = command;
            p.waiters.push(resolve);
          } else {
            q.pending.set(key, { command, waiters: [resolve] });
          }
          q.cancel ??= nextFrame(() => {
            q.cancel = null;
            flush(gesture);
          });
        });
      },
      begin() {
        flush(state.current.gesture);
        state.current = { dragging: true, gesture: null };
      },
      end() {
        const g = state.current.gesture;
        flush(g);
        state.current = { dragging: false, gesture: null };
        if (g !== null) void reportFailure(transport.send(cmd("Edit", { type: "EndGesture", gesture: g })));
      },
      dragging: () => state.current.dragging,
    };
  }, [transport]);
}
