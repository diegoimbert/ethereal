import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Command, TrackId } from "@/generated";
import { useSelectionStore } from "@/state/selection";
import type { EngineTransport } from "@/transport";
import { SELECTION_SYNC_MS, useAgentSelectionSync } from "./selectionSync";

const fakeTransport = () => {
  const sent: Command[] = [];
  const transport = { send: vi.fn((c: Command) => (sent.push(c), Promise.resolve({ type: "Unit" }))) } as unknown as EngineTransport;
  return { transport, sent };
};

afterEach(() => {
  vi.useRealTimers();
  useSelectionStore.setState({ selectedTrack: null });
});

describe("the agent's copy of the selection", () => {
  it("is sent when connected, then after each change settles (once per change)", () => {
    vi.useFakeTimers();
    const { transport, sent } = fakeTransport();
    const { rerender } = renderHook(({ connected }) => useAgentSelectionSync(transport, connected), { initialProps: { connected: false } });
    expect(sent).toHaveLength(0);
    rerender({ connected: true });
    expect(sent).toEqual([{ domain: "Agent", command: { type: "SetSelection", selection: expect.objectContaining({ tracks: [] }) } }]);

    act(() => useSelectionStore.getState().selectTrack("t1" as TrackId));
    act(() => useSelectionStore.getState().selectTrack("t2" as TrackId));
    expect(sent).toHaveLength(1);
    act(() => vi.advanceTimersByTime(SELECTION_SYNC_MS));
    expect(sent).toHaveLength(2);
    expect(sent[1]).toMatchObject({ command: { selection: { tracks: ["t2"] } } });
    // Nothing new: nothing sent.
    act(() => useSelectionStore.getState().selectTrack("t2" as TrackId));
    act(() => vi.advanceTimersByTime(SELECTION_SYNC_MS));
    expect(sent).toHaveLength(2);
  });
});
