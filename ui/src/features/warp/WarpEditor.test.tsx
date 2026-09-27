import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Clip, Command, GestureId, WarpMarker } from "@/generated";
import { useEditorStore, useProjectStore, warpMarkersOfClip } from "@/state";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { WarpEditor } from "./index";

let mock: MockTransport | undefined;
const store = () => useProjectStore.getState();

// jsdom has no canvas: the waveform draws nothing (its geometry is covered by warpMap tests).
beforeEach(() => {
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
});

afterEach(() => {
  vi.restoreAllMocks();
  mock?.dispose();
  mock = undefined;
  store().reset();
  useEditorStore.getState().close();
});

async function flush() {
  await act(async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  });
}

interface Sent {
  command: Command;
  gesture: GestureId | null;
}

/** Mock transport (optionally pretending to be the web build), with every sent command recorded. */
async function setup(opts: { kind?: "wasm"; open?: "audio" | "midi" | null } = {}) {
  mock = new MockTransport({ timers: "manual", seed: 3 });
  if (opts.kind) Object.defineProperty(mock, "kind", { value: opts.kind });
  const sent: Sent[] = [];
  const send = mock.send.bind(mock);
  vi.spyOn(mock, "send").mockImplementation((command, o) => {
    sent.push({ command, gesture: o?.gesture ?? null });
    return send(command, o);
  });
  render(
    <TransportProvider transport={mock}>
      <WarpEditor />
    </TransportProvider>,
  );
  await waitFor(() => expect(store().project).not.toBeNull());
  const clips = Object.values(store().project!.clips);
  const audio = clips.find((c) => c.content.type === "Audio")!;
  const midi = clips.find((c) => c.content.type === "Midi")!;
  const open = opts.open === undefined ? "audio" : opts.open;
  if (open) act(() => useEditorStore.getState().openClip(open === "audio" ? audio.id : midi.id));
  await flush();
  return { clip: audio, sent };
}

const clipNow = (id: string): Clip => store().project!.clips[id]!;
const markers = (clip: string): WarpMarker[] => warpMarkersOfClip(store().project!, clip);
const view = () => screen.getByTestId("warp-view");
const pxPerBeat = () => Number(view().dataset.pxPerBeat);
const markerEl = (id: string) => document.querySelector(`[data-testid="warp-marker"][data-marker-id="${id}"]`)!;

async function addMarker(beat: number) {
  fireEvent.doubleClick(view(), { clientX: beat * pxPerBeat() });
  await flush();
}

describe("WarpEditor", () => {
  it("asks for an audio clip", async () => {
    await setup({ open: null });
    expect(screen.getByTestId("warp-empty").textContent).toMatch(/Double-click an audio clip/);
    act(() => useEditorStore.getState().openClip(Object.values(store().project!.clips).find((c) => c.content.type === "Midi")!.id));
    expect(screen.getByTestId("warp-empty").textContent).toMatch(/MIDI clip/);
  });

  it("shows the clip's warp settings and a waveform", async () => {
    const { clip } = await setup();
    expect(screen.getByTestId("warp-editor").dataset.clipId).toBe(clip.id);
    expect((screen.getByLabelText("Warp") as HTMLInputElement).checked).toBe(true);
    expect((screen.getByLabelText("Warp mode") as HTMLSelectElement).value).toBe("Repitch");
    expect(screen.getByTestId("warp-source-bpm").textContent).toBe("Source 120.00 BPM");
    expect(screen.getByTestId("warp-waveform")).toBeTruthy();
    expect(screen.queryByTestId("warp-web-fallback")).toBeNull();
  });

  it("toggles warp, switches the mode and transposes", async () => {
    const { clip, sent } = await setup();
    fireEvent.change(screen.getByLabelText("Warp mode"), { target: { value: "Complex" } });
    await flush();
    const content = () => clipNow(clip.id).content as Extract<Clip["content"], { type: "Audio" }>;
    expect(content().warp.mode).toBe("Complex");
    fireEvent.click(screen.getByLabelText("Warp"));
    await flush();
    expect(content().warp.enabled).toBe(false);
    expect((screen.getByLabelText("Warp mode") as HTMLSelectElement).disabled).toBe(true);
    expect(screen.getByTestId("warp-marker-count").textContent).toBe("Unwarped");
    fireEvent.change(screen.getByRole("spinbutton", { name: "Transpose" }), { target: { value: "7" } });
    await flush();
    expect(content().transpose).toBe(7);
    expect(sent.map((s) => s.command)).toContainEqual(cmd("Clip", { type: "SetTranspose", id: clip.id, semitones: 7 }));
  });

  it("adds markers on double-click at the snapped beat, pinned to the current source time", async () => {
    const { clip } = await setup();
    await addMarker(4.1); // snaps to 4
    await addMarker(8);
    // 120 BPM source at 120 BPM: beat b plays source b / 2 s.
    expect(markers(clip.id).map((m) => [m.beat, m.source])).toEqual([
      [4, 2],
      [8, 4],
    ]);
    expect(screen.getByTestId("warp-marker-count").textContent).toBe("2 markers");
    // Not while unwarped.
    fireEvent.click(screen.getByLabelText("Warp"));
    await flush();
    await addMarker(12);
    expect(markers(clip.id)).toHaveLength(2);
    expect(document.querySelectorAll('[data-testid="warp-marker"]')).toHaveLength(0);
  });

  it("moves a marker in one undo gesture, between its neighbours", async () => {
    const { clip, sent } = await setup();
    await addMarker(4);
    await addMarker(8);
    const [a, b] = markers(clip.id);
    const px = pxPerBeat();
    sent.length = 0;
    fireEvent.pointerDown(markerEl(b!.id), { button: 0, clientX: 8 * px });
    for (const beat of [7.5, 6, 5]) fireEvent.pointerMove(window, { clientX: beat * px });
    fireEvent.pointerMove(window, { clientX: 1 * px }); // clamped just right of marker a
    fireEvent.pointerUp(window);
    await flush();
    const moved = markers(clip.id).find((m) => m.id === b!.id)!;
    expect(moved.beat).toBeCloseTo(4 + 1 / 64);
    expect(moved.source).toBe(4); // the audio stretches: the source pin stays
    expect(markers(clip.id).find((m) => m.id === a!.id)!.beat).toBe(4);
    const moves = sent.filter((s) => s.command.domain === "Warp");
    expect(moves.length).toBeGreaterThan(1);
    const g = moves[0]!.gesture;
    expect(g).not.toBeNull();
    expect(moves.every((s) => s.gesture === g)).toBe(true);
    expect(sent.at(-1)!.command).toEqual(cmd("Edit", { type: "EndGesture", gesture: g! }));
    // One undo restores the original position.
    await act(async () => {
      await mock!.send(cmd("Edit", { type: "Undo" }));
    });
    expect(markers(clip.id).find((m) => m.id === b!.id)!.beat).toBe(8);
  });

  it("deletes markers by double-click or Delete", async () => {
    const { clip } = await setup();
    await addMarker(2);
    await addMarker(6);
    const [a, b] = markers(clip.id);
    fireEvent.doubleClick(markerEl(a!.id));
    await flush();
    expect(markers(clip.id).map((m) => m.id)).toEqual([b!.id]);
    fireEvent.pointerDown(markerEl(b!.id), { button: 0, clientX: 0 });
    fireEvent.pointerUp(window);
    fireEvent.keyDown(view(), { key: "Delete" });
    await flush();
    expect(markers(clip.id)).toHaveLength(0);
  });

  it("says Complex plays as Repitch in the browser build", async () => {
    await setup({ kind: "wasm" });
    expect(screen.queryByTestId("warp-web-fallback")).toBeNull();
    fireEvent.change(screen.getByLabelText("Warp mode"), { target: { value: "Complex" } });
    await flush();
    expect(screen.getByTestId("warp-web-fallback").textContent).toMatch(/Repitch in the browser/);
  });
});
