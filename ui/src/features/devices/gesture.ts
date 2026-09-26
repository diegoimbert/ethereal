/**
 * Continuous-control plumbing shared by the mixer and the device chain.
 *
 * A drag on a knob/fader is one undo step: every command sent while the pointer is down
 * carries the same `gesture` id, and `Edit::EndGesture` closes it on pointer up. Wire
 * `begin`/`end` to the kit controls' `onChangeStart`/`onChangeEnd`. Changes made outside a
 * drag (keyboard, double-click reset) are sent without a gesture, one undo step each.
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

export function useGestureSender(): GestureSender {
  const transport = useTransport();
  const state = useRef<{ dragging: boolean; gesture: GestureId | null }>({ dragging: false, gesture: null });
  return useMemo<GestureSender>(
    () => ({
      send(command) {
        const s = state.current;
        if (!s.dragging) return reportFailure(transport.send(command));
        if (s.gesture === null) s.gesture = nextGestureId();
        return reportFailure(transport.send(command, { gesture: s.gesture }));
      },
      begin() {
        state.current = { dragging: true, gesture: null };
      },
      end() {
        const g = state.current.gesture;
        state.current = { dragging: false, gesture: null };
        if (g !== null) void reportFailure(transport.send(cmd("Edit", { type: "EndGesture", gesture: g })));
      },
      dragging: () => state.current.dragging,
    }),
    [transport],
  );
}
