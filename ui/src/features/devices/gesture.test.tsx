import { act, renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Command, GestureId, ReplyValue } from "@/generated";
import { cmd } from "@/transport";
import type { EngineTransport, SendOptions } from "@/transport/EngineTransport";
import { TransportContext } from "@/transport/context";
import { useGestureSender } from "./gesture";

/** Records every send; replies Unit. */
function fakeTransport() {
  const sent: { command: Command; gesture: GestureId | null }[] = [];
  const transport = {
    send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
      sent.push({ command, gesture: opts?.gesture ?? null });
      return Promise.resolve({ type: "Unit" } as ReplyValue);
    },
  } as unknown as EngineTransport;
  return { sent, transport };
}

const setParam = (param: number, value: number) => cmd("Device", { type: "SetParam", device: "d1", param, value });

describe("useGestureSender (perf-plugins, audit #4)", () => {
  beforeEach(() => vi.useFakeTimers({ toFake: ["requestAnimationFrame", "cancelAnimationFrame", "setTimeout", "clearTimeout"] }));
  afterEach(() => vi.useRealTimers());

  it("coalesces SetParam per (device, param) per frame during a drag, last value wins", async () => {
    const { sent, transport } = fakeTransport();
    const wrapper = ({ children }: { children: ReactNode }) => (
      <TransportContext.Provider value={{ transport, connection: { state: "connected" } as never }}>{children}</TransportContext.Provider>
    );
    const { result } = renderHook(() => useGestureSender(), { wrapper });
    const g = result.current;

    g.begin();
    const replies = [g.send(setParam(1, 0.1)), g.send(setParam(1, 0.2)), g.send(setParam(2, 0.5)), g.send(setParam(1, 0.3))];
    expect(sent).toHaveLength(0);
    await act(async () => {
      vi.advanceTimersToNextFrame();
    });
    expect(sent.map((s) => s.command)).toEqual([setParam(1, 0.3), setParam(2, 0.5)]);
    const gesture = sent[0]!.gesture;
    expect(gesture).not.toBeNull();
    expect(sent.every((s) => s.gesture === gesture)).toBe(true);
    // Every caller gets the reply of the command that superseded theirs.
    await expect(Promise.all(replies)).resolves.toEqual(Array(4).fill({ type: "Unit" }));

    // Other commands keep their order: what is pending goes first.
    g.send(setParam(1, 0.4));
    g.send(cmd("Device", { type: "SetEnabled", id: "d1", enabled: false }));
    expect(sent.slice(2).map((s) => s.command.command.type)).toEqual(["SetParam", "SetEnabled"]);

    // The end of the drag flushes, then closes the gesture.
    g.send(setParam(1, 0.9));
    g.end();
    expect(sent.slice(4).map((s) => s.command)).toEqual([setParam(1, 0.9), cmd("Edit", { type: "EndGesture", gesture: gesture! })]);

    // Outside a drag: sent at once, no gesture.
    g.send(setParam(1, 1));
    expect(sent.at(-1)).toEqual({ command: setParam(1, 1), gesture: null });
  });
});
